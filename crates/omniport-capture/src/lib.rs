pub mod extcap;
pub mod pcapng;

use bytemuck::{Pod, Zeroable};
use memmap2::{MmapMut, MmapOptions};
use omniport_core::SerialEvent;
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub use extcap::{
    ExtcapError, NamedPipeStreamer, extcap_config_output, extcap_dlts_output,
    extcap_interfaces_output,
};
pub use pcapng::{
    LINKTYPE_RAW, LINKTYPE_SLIP, LINKTYPE_USER0, ParsedPacket, PcapngError, PcapngReader,
    PcapngWriter,
};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct FrameIndexEntry {
    pub timestamp_ns: u64,
    pub data_offset: u64,
    pub payload_len: u32,
    pub endpoint_id: u16,
    pub flags: u8,
    pub pins: u8,
}

const _: () = assert!(std::mem::size_of::<FrameIndexEntry>() == 24);

pub struct MmapCaptureEngine {
    idx_mmap: MmapMut,
    dat_mmap: MmapMut,
    packet_count: AtomicU64,
    write_head_dat: u64,
}

impl MmapCaptureEngine {
    pub fn create(dir: &Path, max_packets: u64, max_data_bytes: u64) -> std::io::Result<Self> {
        let idx_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(dir.join("session.idx"))?;
        idx_file.set_len(max_packets * std::mem::size_of::<FrameIndexEntry>() as u64)?;

        let dat_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(dir.join("session.dat"))?;
        dat_file.set_len(max_data_bytes)?;

        let idx_mmap = unsafe { MmapOptions::new().map_mut(&idx_file)? };
        let dat_mmap = unsafe { MmapOptions::new().map_mut(&dat_file)? };

        Ok(Self {
            idx_mmap,
            dat_mmap,
            packet_count: AtomicU64::new(0),
            write_head_dat: 0,
        })
    }

    #[inline]
    pub fn packet_count(&self) -> u64 {
        self.packet_count.load(Ordering::Acquire)
    }

    pub fn append_frame(&mut self, entry: FrameIndexEntry, payload: &[u8]) {
        let pkt_idx = self.packet_count.load(Ordering::Relaxed) as usize;
        let idx_stride = std::mem::size_of::<FrameIndexEntry>();
        let idx_byte_offset = pkt_idx * idx_stride;

        let dat_offset = self.write_head_dat as usize;
        self.dat_mmap[dat_offset..dat_offset + payload.len()].copy_from_slice(payload);
        self.write_head_dat += payload.len() as u64;

        let mut finalized_entry = entry;
        finalized_entry.data_offset = dat_offset as u64;
        finalized_entry.payload_len = payload.len() as u32;

        let entry_bytes = bytemuck::bytes_of(&finalized_entry);
        self.idx_mmap[idx_byte_offset..idx_byte_offset + idx_stride].copy_from_slice(entry_bytes);

        self.packet_count
            .store((pkt_idx + 1) as u64, Ordering::Release);
    }

    /// Appends a SerialEvent directly into the dual-mmap capture store
    pub fn append_event(&mut self, event: &SerialEvent) {
        let entry = FrameIndexEntry {
            timestamp_ns: event.timestamp_ns,
            data_offset: 0,
            payload_len: event.payload.len() as u32,
            endpoint_id: event.source_id,
            flags: event.flags.bits(),
            pins: event.pins.bits(),
        };
        self.append_frame(entry, &event.payload);
    }

    /// O(1) viewport reader for virtualized GUI row rendering
    pub fn read_viewport(&self, start_row: u64, row_count: usize, out_buf: &mut Vec<u8>) {
        let total = self.packet_count.load(Ordering::Acquire);
        if start_row >= total {
            out_buf.extend_from_slice(&total.to_le_bytes());
            out_buf.extend_from_slice(&0u32.to_le_bytes());
            out_buf.extend_from_slice(&0u32.to_le_bytes());
            return;
        }
        let end_row = (start_row + row_count as u64).min(total);
        let idx_stride = std::mem::size_of::<FrameIndexEntry>();

        out_buf.extend_from_slice(&total.to_le_bytes());
        out_buf.extend_from_slice(&((end_row - start_row) as u32).to_le_bytes());
        out_buf.extend_from_slice(&0u32.to_le_bytes()); // 4B alignment padding per ICD

        for row in start_row..end_row {
            let off = (row as usize) * idx_stride;
            let entry_slice = &self.idx_mmap[off..off + idx_stride];
            let entry: &FrameIndexEntry = bytemuck::from_bytes(entry_slice);

            out_buf.extend_from_slice(entry_slice);
            let d_start = entry.data_offset as usize;
            let d_end = d_start + (entry.payload_len as usize);
            out_buf.extend_from_slice(&self.dat_mmap[d_start..d_end]);
        }
    }

    /// Exports the entire recorded capture session to standard PCAPNG
    pub fn export_pcapng<W: std::io::Write>(
        &self,
        writer: W,
        link_type: u16,
    ) -> Result<u64, PcapngError> {
        let mut pcapng = PcapngWriter::with_link_type(writer, link_type);
        pcapng.write_header("OmniPort Session Export")?;

        let total = self.packet_count();
        let idx_stride = std::mem::size_of::<FrameIndexEntry>();

        for row in 0..total {
            let off = (row as usize) * idx_stride;
            let entry_slice = &self.idx_mmap[off..off + idx_stride];
            let entry: &FrameIndexEntry = bytemuck::from_bytes(entry_slice);

            let d_start = entry.data_offset as usize;
            let d_end = d_start + (entry.payload_len as usize);
            let payload = &self.dat_mmap[d_start..d_end];

            pcapng.write_packet(entry.timestamp_ns, payload, entry.flags)?;
        }
        pcapng.flush()?;
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omniport_core::{FrameFlags, PinState};
    use std::time::Instant;
    use tempfile::tempdir;

    #[test]
    fn test_o1_mmap_append_and_viewport_read() {
        let dir = tempdir().unwrap();
        let mut engine = MmapCaptureEngine::create(dir.path(), 1024, 65536).unwrap();

        let entry = FrameIndexEntry {
            timestamp_ns: 1_000_000,
            data_offset: 0,
            payload_len: 0,
            endpoint_id: 1,
            flags: 1,
            pins: 0b0000_0101,
        };
        engine.append_frame(entry, b"\xAA\x55\x01\x02");

        let mut wire = Vec::new();
        engine.read_viewport(0, 10, &mut wire);

        let total = u64::from_le_bytes(wire[0..8].try_into().unwrap());
        let count = u32::from_le_bytes(wire[8..12].try_into().unwrap());
        assert_eq!(total, 1);
        assert_eq!(count, 1);
        assert_eq!(&wire[16 + 24..16 + 28], b"\xAA\x55\x01\x02");
    }

    #[test]
    fn test_append_event_and_pcapng_export() {
        let dir = tempdir().unwrap();
        let mut engine = MmapCaptureEngine::create(dir.path(), 100, 4096).unwrap();

        for i in 0..5 {
            let evt = SerialEvent {
                timestamp_ns: 1_000_000_000 + i * 10_000,
                source_id: 1,
                flags: FrameFlags::empty(),
                pins: PinState::RTS | PinState::DTR,
                payload: bytes::Bytes::from(format!("EVENT_{}", i)),
            };
            engine.append_event(&evt);
        }

        assert_eq!(engine.packet_count(), 5);

        let mut pcapng_bytes = Vec::new();
        let exported = engine
            .export_pcapng(&mut pcapng_bytes, LINKTYPE_USER0)
            .unwrap();
        assert_eq!(exported, 5);

        let mut reader = PcapngReader::new(pcapng_bytes.as_slice());
        let packets = reader.read_all_packets().unwrap();
        assert_eq!(packets.len(), 5);
        for (i, pkt) in packets.iter().enumerate() {
            assert_eq!(pkt.payload, format!("EVENT_{}", i).as_bytes());
        }
    }

    /// FR-401 / VV_RTM: O(1) viewport seek latency verification (< 100 microseconds)
    #[test]
    fn test_o1_viewport_seek_latency() {
        let dir = tempdir().unwrap();
        let max_packets = 50_000u64;
        let mut engine =
            MmapCaptureEngine::create(dir.path(), max_packets, max_packets * 16).unwrap();

        for i in 0..max_packets {
            let entry = FrameIndexEntry {
                timestamp_ns: 1_000_000 + i,
                data_offset: 0,
                payload_len: 4,
                endpoint_id: 1,
                flags: 0,
                pins: 0,
            };
            engine.append_frame(entry, b"SEEK");
        }

        assert_eq!(engine.packet_count(), max_packets);

        // Measure random seeks across the 50,000 packet store
        let seek_targets = [0, 1000, 25000, 40000, 49950];
        let mut buf = Vec::with_capacity(4096);

        for &row in &seek_targets {
            buf.clear();
            let start = Instant::now();
            engine.read_viewport(row, 20, &mut buf);
            let elapsed = start.elapsed();

            // Target criteria: seek latency < 100 microseconds
            assert!(
                elapsed.as_micros() < 100,
                "Seek to row {} took {:?}, exceeding 100 us threshold",
                row,
                elapsed
            );

            let returned = u32::from_le_bytes(buf[8..12].try_into().unwrap());
            assert_eq!(returned, 20);
        }
    }
}
