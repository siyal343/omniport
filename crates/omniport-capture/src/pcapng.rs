use omniport_core::SerialEvent;
use std::io::{Read, Write};
use thiserror::Error;

pub const BLOCK_TYPE_SHB: u32 = 0x0A0D_0D0A;
pub const BLOCK_TYPE_IDB: u32 = 0x0000_0001;
pub const BLOCK_TYPE_EPB: u32 = 0x0000_0006;

pub const BYTE_ORDER_MAGIC: u32 = 0x1A2E_3D4C;
pub const LINKTYPE_USER0: u16 = 147;
pub const LINKTYPE_RAW: u16 = 101;
pub const LINKTYPE_SLIP: u16 = 8;
pub const TS_RESOL_NANO: u8 = 9; // 10^-9 ns timestamp resolution

/// Upper bound on a single PCAPNG block we are willing to buffer. OmniPort serial frames
/// are far smaller than this; the cap keeps a malformed or hostile capture from forcing a
/// multi-gigabyte allocation before failing.
pub const MAX_BLOCK_LEN: u32 = 16 * 1024 * 1024;

/// Size of an Enhanced Packet Block body excluding the packet data and padding
/// (InterfaceId, TsHigh, TsLow, CapLen, OrigLen).
const EPB_FIXED_BODY_LEN: usize = 20;

#[derive(Error, Debug)]
pub enum PcapngError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid PCAPNG byte-order magic: {0:#010X}")]
    InvalidMagic(u32),
    #[error("Invalid block total length: {0}")]
    InvalidBlockLength(u32),
    #[error("Block total length mismatch between header ({0}) and trailer ({1})")]
    LengthMismatch(u32, u32),
    #[error("Unexpected end of PCAPNG stream")]
    UnexpectedEof,
    #[error("Unsupported or invalid block type: {0:#010X}")]
    UnsupportedBlockType(u32),
}

/// Serializes live serial events or recorded captures into standard PCAPNG format
pub struct PcapngWriter<W: Write> {
    writer: W,
    link_type: u16,
    ts_resol_power: u8,
    header_written: bool,
}

impl<W: Write> PcapngWriter<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            link_type: LINKTYPE_USER0,
            ts_resol_power: TS_RESOL_NANO,
            header_written: false,
        }
    }

    pub fn with_link_type(writer: W, link_type: u16) -> Self {
        Self {
            writer,
            link_type,
            ts_resol_power: TS_RESOL_NANO,
            header_written: false,
        }
    }

    /// Writes Section Header Block (SHB) and Interface Description Block (IDB)
    pub fn write_header(&mut self, if_name: &str) -> Result<(), PcapngError> {
        if self.header_written {
            return Ok(());
        }

        // 1. Build Section Header Block (SHB)
        // Option 3 (shb_userappl) = "OmniPort", padded to a 32-bit boundary, then
        // Option 0 (end of options).
        let mut shb_options = Vec::new();
        let app_name = b"OmniPort";
        shb_options.extend_from_slice(&3u16.to_le_bytes()); // opt_code: shb_userappl
        shb_options.extend_from_slice(&(app_name.len() as u16).to_le_bytes()); // opt_len
        shb_options.extend_from_slice(app_name);
        let app_pad = (4 - (app_name.len() % 4)) % 4;
        shb_options.resize(shb_options.len() + app_pad, 0);
        // Option 0: End of options
        shb_options.extend_from_slice(&0u16.to_le_bytes());
        shb_options.extend_from_slice(&0u16.to_le_bytes());

        let shb_body_len = 16 + shb_options.len(); // Magic (4) + Maj (2) + Min (2) + SecLen (8) + opts
        let shb_total_len = (8 + shb_body_len + 4) as u32; // Type (4) + Len1 (4) + body + Len2 (4)

        self.writer.write_all(&BLOCK_TYPE_SHB.to_le_bytes())?;
        self.writer.write_all(&shb_total_len.to_le_bytes())?;
        self.writer.write_all(&BYTE_ORDER_MAGIC.to_le_bytes())?;
        self.writer.write_all(&1u16.to_le_bytes())?; // Major 1
        self.writer.write_all(&0u16.to_le_bytes())?; // Minor 0
        self.writer.write_all(&(-1i64).to_le_bytes())?; // Section length: -1 (unspecified/streamed)
        self.writer.write_all(&shb_options)?;
        self.writer.write_all(&shb_total_len.to_le_bytes())?;

        // 2. Build Interface Description Block (IDB)
        let mut idb_options = Vec::new();

        // Option 2: if_name
        let if_name_bytes = if_name.as_bytes();
        idb_options.extend_from_slice(&2u16.to_le_bytes());
        idb_options.extend_from_slice(&(if_name_bytes.len() as u16).to_le_bytes());
        idb_options.extend_from_slice(if_name_bytes);
        let name_pad = (4 - (if_name_bytes.len() % 4)) % 4;
        idb_options.resize(idb_options.len() + name_pad, 0);

        // Option 9: if_tsresol (timestamp resolution = 10^-9 s for nanoseconds)
        idb_options.extend_from_slice(&9u16.to_le_bytes());
        idb_options.extend_from_slice(&1u16.to_le_bytes());
        idb_options.push(self.ts_resol_power);
        idb_options.extend_from_slice(&[0, 0, 0]); // 3B padding to 4B boundary

        // Option 0: End of options
        idb_options.extend_from_slice(&0u16.to_le_bytes());
        idb_options.extend_from_slice(&0u16.to_le_bytes());

        let idb_body_len = 8 + idb_options.len(); // LinkType (2) + Reserved (2) + SnapLen (4) + opts
        let idb_total_len = (8 + idb_body_len + 4) as u32;

        self.writer.write_all(&BLOCK_TYPE_IDB.to_le_bytes())?;
        self.writer.write_all(&idb_total_len.to_le_bytes())?;
        self.writer.write_all(&self.link_type.to_le_bytes())?;
        self.writer.write_all(&0u16.to_le_bytes())?; // Reserved
        self.writer.write_all(&65535u32.to_le_bytes())?; // SnapLen
        self.writer.write_all(&idb_options)?;
        self.writer.write_all(&idb_total_len.to_le_bytes())?;

        self.header_written = true;
        self.writer.flush()?;
        Ok(())
    }

    /// Writes an Enhanced Packet Block (EPB) for a single packet payload
    pub fn write_packet(
        &mut self,
        timestamp_ns: u64,
        payload: &[u8],
        _flags: u8,
    ) -> Result<(), PcapngError> {
        if !self.header_written {
            self.write_header("OmniPort Virtual Serial")?;
        }

        let cap_len = payload.len() as u32;
        let orig_len = cap_len;
        let pad_len = (4 - (payload.len() % 4)) % 4;

        // EPB layout:
        // Type (4) + TotalLen1 (4)
        // InterfaceId (4) + TsHigh (4) + TsLow (4) + CapLen (4) + OrigLen (4)
        // Packet Data (cap_len) + Pad (pad_len)
        // TotalLen2 (4)
        let epb_total_len = (32 + payload.len() + pad_len) as u32;

        let ts_high = (timestamp_ns >> 32) as u32;
        let ts_low = (timestamp_ns & 0xFFFF_FFFF) as u32;

        self.writer.write_all(&BLOCK_TYPE_EPB.to_le_bytes())?;
        self.writer.write_all(&epb_total_len.to_le_bytes())?;
        self.writer.write_all(&0u32.to_le_bytes())?; // Interface ID 0
        self.writer.write_all(&ts_high.to_le_bytes())?;
        self.writer.write_all(&ts_low.to_le_bytes())?;
        self.writer.write_all(&cap_len.to_le_bytes())?;
        self.writer.write_all(&orig_len.to_le_bytes())?;
        self.writer.write_all(payload)?;

        if pad_len > 0 {
            self.writer.write_all(&[0u8, 0, 0][..pad_len])?;
        }

        self.writer.write_all(&epb_total_len.to_le_bytes())?;
        Ok(())
    }

    pub fn write_event(&mut self, event: &SerialEvent) -> Result<(), PcapngError> {
        self.write_packet(event.timestamp_ns, &event.payload, event.flags.bits())
    }

    pub fn flush(&mut self) -> Result<(), PcapngError> {
        self.writer.flush()?;
        Ok(())
    }

    pub fn into_inner(self) -> W {
        self.writer
    }
}

/// A parsed packet from a PCAPNG file or live stream
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPacket {
    pub timestamp_ns: u64,
    pub interface_id: u32,
    pub payload: Vec<u8>,
    pub original_len: u32,
}

/// Deserializer and validator for PCAPNG formatted data
pub struct PcapngReader<R: Read> {
    reader: R,
}

impl<R: Read> PcapngReader<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    /// Reads and parses all packets from the PCAPNG stream
    pub fn read_all_packets(&mut self) -> Result<Vec<ParsedPacket>, PcapngError> {
        let mut packets = Vec::new();
        loop {
            let mut block_hdr = [0u8; 8];
            match self.reader.read_exact(&mut block_hdr) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(PcapngError::Io(e)),
            }

            let block_type =
                u32::from_le_bytes([block_hdr[0], block_hdr[1], block_hdr[2], block_hdr[3]]);
            let total_len =
                u32::from_le_bytes([block_hdr[4], block_hdr[5], block_hdr[6], block_hdr[7]]);

            if total_len < 12 {
                return Err(PcapngError::InvalidBlockLength(total_len));
            }
            if total_len > MAX_BLOCK_LEN {
                return Err(PcapngError::InvalidBlockLength(total_len));
            }

            let body_len = (total_len - 12) as usize;
            let mut body_and_trailer = vec![0u8; body_len + 4];
            self.reader.read_exact(&mut body_and_trailer)?;

            let trailer_len = u32::from_le_bytes([
                body_and_trailer[body_len],
                body_and_trailer[body_len + 1],
                body_and_trailer[body_len + 2],
                body_and_trailer[body_len + 3],
            ]);

            if trailer_len != total_len {
                return Err(PcapngError::LengthMismatch(total_len, trailer_len));
            }

            if block_type == BLOCK_TYPE_EPB {
                // The fixed header fields must fit in the block body.
                if body_len < EPB_FIXED_BODY_LEN {
                    return Err(PcapngError::InvalidBlockLength(total_len));
                }
                let if_id = u32::from_le_bytes(body_and_trailer[0..4].try_into().unwrap());
                let ts_high = u32::from_le_bytes(body_and_trailer[4..8].try_into().unwrap());
                let ts_low = u32::from_le_bytes(body_and_trailer[8..12].try_into().unwrap());
                let cap_len =
                    u32::from_le_bytes(body_and_trailer[12..16].try_into().unwrap()) as usize;
                let orig_len = u32::from_le_bytes(body_and_trailer[16..20].try_into().unwrap());

                let payload_end = EPB_FIXED_BODY_LEN + cap_len;
                if payload_end > body_len {
                    return Err(PcapngError::InvalidBlockLength(total_len));
                }

                let timestamp_ns = ((ts_high as u64) << 32) | (ts_low as u64);
                let payload = body_and_trailer[EPB_FIXED_BODY_LEN..payload_end].to_vec();

                packets.push(ParsedPacket {
                    timestamp_ns,
                    interface_id: if_id,
                    payload,
                    original_len: orig_len,
                });
            }
        }
        Ok(packets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shb_userappl_option_uses_spec_option_code_3() {
        let mut buf = Vec::new();
        {
            let mut writer = PcapngWriter::new(&mut buf);
            writer.write_header("test_port").unwrap();
            writer.flush().unwrap();
        }

        // SHB body starts after the 8-byte block header. Layout:
        // ByteOrderMagic (4) + Major (2) + Minor (2) + SectionLength (8) + options.
        let total_len = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
        let options = &buf[24..total_len - 4];
        let opt_code = u16::from_le_bytes(options[0..2].try_into().unwrap());
        let opt_len = u16::from_le_bytes(options[2..4].try_into().unwrap()) as usize;
        // PCAPNG spec: shb_userappl option code is 3 (1 = shb_hardware, 2 = shb_os).
        assert_eq!(opt_code, 3, "shb_userappl must use option code 3");
        assert_eq!(&options[4..4 + opt_len], b"OmniPort");
    }

    fn block(block_type: u32, total_len: u32, body_and_trailer: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&block_type.to_le_bytes());
        bytes.extend_from_slice(&total_len.to_le_bytes());
        bytes.extend_from_slice(body_and_trailer);
        bytes
    }

    #[test]
    fn test_reader_rejects_truncated_epb_without_panicking() {
        // EPB with total_len == 12 => zero body bytes. Parsing must report a malformed
        // block instead of indexing past the body buffer.
        let bytes = block(BLOCK_TYPE_EPB, 12, &12u32.to_le_bytes());
        let mut reader = PcapngReader::new(bytes.as_slice());
        let result = reader.read_all_packets();
        assert!(matches!(result, Err(PcapngError::InvalidBlockLength(12))));
    }

    #[test]
    fn test_reader_rejects_capture_len_overrun_without_panicking() {
        // EPB whose captured length claims more payload bytes than the block contains.
        let mut body = Vec::new();
        body.extend_from_slice(&0u32.to_le_bytes()); // interface id
        body.extend_from_slice(&0u32.to_le_bytes()); // ts high
        body.extend_from_slice(&0u32.to_le_bytes()); // ts low
        body.extend_from_slice(&1000u32.to_le_bytes()); // captured length (bogus)
        body.extend_from_slice(&1000u32.to_le_bytes()); // original length
        body.extend_from_slice(&32u32.to_le_bytes()); // trailer
        let bytes = block(BLOCK_TYPE_EPB, 32, &body);
        assert_eq!(bytes.len(), 32);

        let mut reader = PcapngReader::new(bytes.as_slice());
        let result = reader.read_all_packets();
        assert!(
            matches!(result, Err(PcapngError::InvalidBlockLength(32))),
            "expected InvalidBlockLength, got {result:?}"
        );
    }

    #[test]
    fn test_reader_rejects_absurd_block_length_without_allocating() {
        // A malformed block claiming a 20 MB total length must be rejected by the
        // length sanity cap rather than allocating the full body buffer.
        let bytes = block(BLOCK_TYPE_EPB, 20_000_000, &[]);
        let mut reader = PcapngReader::new(bytes.as_slice());
        let result = reader.read_all_packets();
        assert!(
            matches!(result, Err(PcapngError::InvalidBlockLength(20_000_000))),
            "expected InvalidBlockLength, got {result:?}"
        );
    }

    #[test]
    fn test_pcapng_write_and_read_roundtrip() {
        let mut buf = Vec::new();
        let mut writer = PcapngWriter::new(&mut buf);
        writer.write_header("test_port").unwrap();

        let packets = [
            (1_000_000_000u64, b"HELLO_SERIAL_1".as_slice()),
            (1_000_050_000u64, b"EXT_PACKET_2_ODD_LEN".as_slice()),
            (1_000_100_000u64, b"X".as_slice()),
            (1_000_200_000u64, b"".as_slice()),
        ];

        for &(ts, payload) in &packets {
            writer.write_packet(ts, payload, 0).unwrap();
        }
        writer.flush().unwrap();

        assert!(!buf.is_empty());

        let mut reader = PcapngReader::new(buf.as_slice());
        let read_packets = reader.read_all_packets().unwrap();

        assert_eq!(read_packets.len(), packets.len());
        for (i, (&(exp_ts, exp_payload), actual)) in
            packets.iter().zip(read_packets.iter()).enumerate()
        {
            assert_eq!(
                actual.timestamp_ns, exp_ts,
                "Packet {} timestamp mismatch",
                i
            );
            assert_eq!(
                &actual.payload[..],
                exp_payload,
                "Packet {} payload mismatch",
                i
            );
            assert_eq!(actual.original_len as usize, exp_payload.len());
        }
    }
}
