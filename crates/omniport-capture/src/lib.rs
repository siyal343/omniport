use bytemuck::{Pod, Zeroable};
use memmap2::{MmapMut, MmapOptions};
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

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
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
