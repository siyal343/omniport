use crate::crc::CrcAlgorithm;
use crate::{EndpointId, FrameFlags, PinState, SerialEvent};
use bytes::{Buf, Bytes, BytesMut};
use std::time::Duration;

pub const SLIP_END: u8 = 0xC0;
pub const SLIP_ESC: u8 = 0xDB;
pub const SLIP_ESC_END: u8 = 0xDC;
pub const SLIP_ESC_ESC: u8 = 0xDD;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FramingRule {
    Raw,
    Delimiter(Vec<u8>),
    Cobs,
    Slip,
    LengthField {
        sync_word: Vec<u8>,
        len_offset: usize,
        len_bytes: u8,
        little_endian: bool,
        header_overhead: usize,
    },
    IdleGap(Duration),
}

pub struct StreamPacketizer {
    rule: FramingRule,
    crc: Option<CrcAlgorithm>,
    strip_crc: bool,
    rx_buf: BytesMut,
    last_rx_ns: u64,
}

impl StreamPacketizer {
    pub fn new(rule: FramingRule) -> Self {
        Self {
            rule,
            crc: None,
            strip_crc: false,
            rx_buf: BytesMut::with_capacity(8192),
            last_rx_ns: 0,
        }
    }

    pub fn with_crc(mut self, crc: CrcAlgorithm) -> Self {
        self.crc = Some(crc);
        self
    }

    pub fn with_strip_crc(mut self, strip_crc: bool) -> Self {
        self.strip_crc = strip_crc;
        self
    }

    pub fn set_crc(&mut self, crc: Option<CrcAlgorithm>) {
        self.crc = crc;
    }

    pub fn set_strip_crc(&mut self, strip_crc: bool) {
        self.strip_crc = strip_crc;
    }

    pub fn crc(&self) -> Option<&CrcAlgorithm> {
        self.crc.as_ref()
    }

    pub fn strip_crc(&self) -> bool {
        self.strip_crc
    }

    pub fn rule(&self) -> &FramingRule {
        &self.rule
    }

    pub fn rx_buf(&self) -> &[u8] {
        &self.rx_buf
    }

    pub fn clear(&mut self) {
        self.rx_buf.clear();
        self.last_rx_ns = 0;
    }

    /// Emits a single frame to the output buffer, verifying and optionally stripping CRC if configured.
    fn emit_frame(&self, frame: Bytes, out: &mut Vec<Bytes>) {
        if let Some(crc) = &self.crc {
            if crc.validate(&frame) {
                if self.strip_crc {
                    let payload_len = frame.len() - crc.checksum_len();
                    out.push(frame.slice(0..payload_len));
                } else {
                    out.push(frame);
                }
            }
        } else {
            out.push(frame);
        }
    }

    /// Processes an incoming raw byte chunk.
    pub fn push_chunk(&mut self, chunk: &[u8], now_ns: u64, out: &mut Vec<Bytes>) {
        if let FramingRule::IdleGap(gap) = self.rule {
            let gap_ns = gap.as_nanos() as u64;
            if !self.rx_buf.is_empty() && now_ns.saturating_sub(self.last_rx_ns) >= gap_ns {
                let frame = self.rx_buf.split().freeze();
                self.emit_frame(frame, out);
            }
        }
        self.last_rx_ns = now_ns;

        if matches!(self.rule, FramingRule::Raw) {
            let frame = Bytes::copy_from_slice(chunk);
            self.emit_frame(frame, out);
            return;
        }

        self.rx_buf.extend_from_slice(chunk);
        self.extract_frames(out);
    }

    /// Triggers Idle-Gap timeout check. If elapsed time since last byte exceeds the gap, emits pending frame.
    pub fn check_timeout(&mut self, now_ns: u64, out: &mut Vec<Bytes>) {
        if let FramingRule::IdleGap(gap) = self.rule {
            let gap_ns = gap.as_nanos() as u64;
            if !self.rx_buf.is_empty() && now_ns.saturating_sub(self.last_rx_ns) >= gap_ns {
                let frame = self.rx_buf.split().freeze();
                self.emit_frame(frame, out);
            }
        }
    }

    /// Flushes any pending data in the receive buffer as a frame.
    pub fn flush(&mut self, out: &mut Vec<Bytes>) {
        if !self.rx_buf.is_empty() {
            let frame = self.rx_buf.split().freeze();
            self.emit_frame(frame, out);
        }
    }

    /// Processes incoming chunks and outputs `SerialEvent` items with `FrameFlags::ERR_CRC` flagged if CRC check fails.
    pub fn push_chunk_events(
        &mut self,
        chunk: &[u8],
        now_ns: u64,
        source_id: EndpointId,
        out: &mut Vec<SerialEvent>,
    ) {
        let mut raw_frames = Vec::new();
        // Temporarily disable CRC filtering to capture corrupted frames as events
        let saved_crc = self.crc.take();
        self.push_chunk(chunk, now_ns, &mut raw_frames);
        self.crc = saved_crc;

        for frame in raw_frames {
            let mut flags = FrameFlags::empty();
            let payload = if let Some(crc) = &self.crc {
                if !crc.validate(&frame) {
                    flags |= FrameFlags::ERR_CRC;
                    frame
                } else if self.strip_crc {
                    let payload_len = frame.len() - crc.checksum_len();
                    frame.slice(0..payload_len)
                } else {
                    frame
                }
            } else {
                frame
            };

            out.push(SerialEvent {
                timestamp_ns: now_ns,
                source_id,
                flags,
                pins: PinState::empty(),
                payload,
            });
        }
    }

    fn extract_frames(&mut self, out: &mut Vec<Bytes>) {
        match &self.rule {
            FramingRule::Delimiter(delim) => {
                while let Some(pos) = self.rx_buf.windows(delim.len()).position(|w| w == delim) {
                    let frame = self.rx_buf.split_to(pos + delim.len()).freeze();
                    self.emit_frame(frame, out);
                }
            }
            FramingRule::Cobs => {
                while let Some(pos) = self.rx_buf.iter().position(|&b| b == 0x00) {
                    let raw_frame = self.rx_buf.split_to(pos + 1);
                    if let Some(decoded) = Self::decode_cobs(&raw_frame[..pos]) {
                        self.emit_frame(Bytes::from(decoded), out);
                    }
                }
            }
            FramingRule::Slip => {
                while let Some(pos) = self.rx_buf.iter().position(|&b| b == SLIP_END) {
                    let raw_frame = self.rx_buf.split_to(pos + 1);
                    let frame_bytes = &raw_frame[..pos];
                    if !frame_bytes.is_empty()
                        && let Some(decoded) = Self::decode_slip(frame_bytes)
                    {
                        self.emit_frame(Bytes::from(decoded), out);
                    }
                }
            }
            FramingRule::LengthField {
                sync_word,
                len_offset,
                len_bytes,
                little_endian,
                header_overhead,
            } => loop {
                if !sync_word.is_empty() {
                    if let Some(sync_pos) = self
                        .rx_buf
                        .windows(sync_word.len())
                        .position(|w| w == sync_word)
                    {
                        if sync_pos > 0 {
                            self.rx_buf.advance(sync_pos);
                        }
                    } else {
                        break;
                    }
                }
                let min_hdr = len_offset + (*len_bytes as usize);
                if self.rx_buf.len() < min_hdr {
                    break;
                }
                let payload_len = match (*len_bytes, *little_endian) {
                    (1, _) => self.rx_buf[*len_offset] as usize,
                    (2, true) => {
                        u16::from_le_bytes([self.rx_buf[*len_offset], self.rx_buf[*len_offset + 1]])
                            as usize
                    }
                    (2, false) => {
                        u16::from_be_bytes([self.rx_buf[*len_offset], self.rx_buf[*len_offset + 1]])
                            as usize
                    }
                    (4, true) => u32::from_le_bytes([
                        self.rx_buf[*len_offset],
                        self.rx_buf[*len_offset + 1],
                        self.rx_buf[*len_offset + 2],
                        self.rx_buf[*len_offset + 3],
                    ]) as usize,
                    (4, false) => u32::from_be_bytes([
                        self.rx_buf[*len_offset],
                        self.rx_buf[*len_offset + 1],
                        self.rx_buf[*len_offset + 2],
                        self.rx_buf[*len_offset + 3],
                    ]) as usize,
                    _ => 0,
                };
                let total_frame_len = payload_len + *header_overhead;
                if self.rx_buf.len() < total_frame_len {
                    break;
                }
                let frame = self.rx_buf.split_to(total_frame_len).freeze();
                self.emit_frame(frame, out);
            },
            FramingRule::IdleGap(_) | FramingRule::Raw => {}
        }
    }

    /// Decodes a COBS-encoded byte sequence without the terminating zero.
    pub fn decode_cobs(input: &[u8]) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(input.len());
        let mut idx = 0;
        while idx < input.len() {
            let code = input[idx] as usize;
            if code == 0 || idx + code > input.len() + 1 {
                return None;
            }
            idx += 1;
            out.extend_from_slice(&input[idx..idx + code - 1]);
            idx += code - 1;
            if code < 0xFF && idx < input.len() {
                out.push(0x00);
            }
        }
        Some(out)
    }

    /// Encodes arbitrary bytes into COBS framing with a trailing 0x00 delimiter.
    pub fn encode_cobs(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(payload.len() + payload.len() / 254 + 2);
        let mut code_idx = 0;
        out.push(0x01);
        let mut code: u8 = 0x01;

        for &b in payload {
            if b == 0x00 {
                out[code_idx] = code;
                code_idx = out.len();
                out.push(0x01);
                code = 0x01;
            } else {
                out.push(b);
                code += 1;
                if code == 0xFF {
                    out[code_idx] = code;
                    code_idx = out.len();
                    out.push(0x01);
                    code = 0x01;
                }
            }
        }
        out[code_idx] = code;
        out.push(0x00);
        out
    }

    /// Decodes an RFC 1055 SLIP-encoded frame without the bounding 0xC0 delimiters.
    pub fn decode_slip(input: &[u8]) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(input.len());
        let mut i = 0;
        while i < input.len() {
            match input[i] {
                SLIP_ESC => {
                    i += 1;
                    if i >= input.len() {
                        return None;
                    }
                    match input[i] {
                        SLIP_ESC_END => out.push(SLIP_END),
                        SLIP_ESC_ESC => out.push(SLIP_ESC),
                        other => out.push(other),
                    }
                }
                b => out.push(b),
            }
            i += 1;
        }
        Some(out)
    }

    /// Encodes arbitrary bytes into an RFC 1055 SLIP-framed byte vector with bounding 0xC0 delimiters.
    pub fn encode_slip(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(payload.len() + 8);
        out.push(SLIP_END);
        for &b in payload {
            match b {
                SLIP_END => {
                    out.push(SLIP_ESC);
                    out.push(SLIP_ESC_END);
                }
                SLIP_ESC => {
                    out.push(SLIP_ESC);
                    out.push(SLIP_ESC_ESC);
                }
                _ => out.push(b),
            }
        }
        out.push(SLIP_END);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delimiter_framing_across_fragmented_chunks() {
        let mut p = StreamPacketizer::new(FramingRule::Delimiter(b"\r\n".to_vec()));
        let mut out = Vec::new();
        p.push_chunk(b"$GPGGA,123", 100, &mut out);
        assert!(out.is_empty());
        p.push_chunk(b"456\r\n$GPGLL", 200, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(&out[0][..], b"$GPGGA,123456\r\n");
    }

    #[test]
    fn test_cobs_roundtrip() {
        let payload = vec![0x11, 0x22, 0x00, 0x33, 0x00, 0x44, 0x55];
        let encoded = StreamPacketizer::encode_cobs(&payload);
        assert_eq!(*encoded.last().unwrap(), 0x00);

        let mut p = StreamPacketizer::new(FramingRule::Cobs);
        let mut out = Vec::new();
        // Fragment across two chunks
        p.push_chunk(&encoded[..3], 100, &mut out);
        assert!(out.is_empty());
        p.push_chunk(&encoded[3..], 200, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(&out[0][..], &payload[..]);
    }

    #[test]
    fn test_slip_framing_and_escaping() {
        let payload = vec![0x01, SLIP_END, 0x02, SLIP_ESC, 0x03];
        let encoded = StreamPacketizer::encode_slip(&payload);

        let mut p = StreamPacketizer::new(FramingRule::Slip);
        let mut out = Vec::new();
        // Push in small 2-byte fragments to test escape splitting
        for chunk in encoded.chunks(2) {
            p.push_chunk(chunk, 100, &mut out);
        }
        assert_eq!(out.len(), 1);
        assert_eq!(&out[0][..], &payload[..]);
    }

    #[test]
    fn test_slip_consecutive_delimiters_ignored() {
        let mut p = StreamPacketizer::new(FramingRule::Slip);
        let mut out = Vec::new();
        // Multiple delimiters before and after
        p.push_chunk(
            &[SLIP_END, SLIP_END, 0x10, 0x20, SLIP_END, SLIP_END],
            100,
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(&out[0][..], &[0x10, 0x20]);
    }

    #[test]
    fn test_idle_gap_timer_framing() {
        let gap = Duration::from_millis(10);
        let mut p = StreamPacketizer::new(FramingRule::IdleGap(gap));
        let mut out = Vec::new();

        // Send first chunk at t=0 ms (0 ns)
        p.push_chunk(b"hello", 0, &mut out);
        assert!(out.is_empty());

        // Send second chunk at t=5 ms (5_000_000 ns) - within 10 ms gap
        p.push_chunk(b" world", 5_000_000, &mut out);
        assert!(out.is_empty());

        // At t=12 ms (12_000_000 ns) - gap since t=5 ms is only 7 ms (<10 ms)
        p.check_timeout(12_000_000, &mut out);
        assert!(out.is_empty());

        // At t=16 ms (16_000_000 ns) - gap since t=5 ms is 11 ms (>= 10 ms) -> fires!
        p.check_timeout(16_000_000, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(&out[0][..], b"hello world");

        // Subsequent chunk starts a new frame
        p.push_chunk(b"next frame", 20_000_000, &mut out);
        p.flush(&mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(&out[1][..], b"next frame");
    }

    #[test]
    fn test_crc_validation_filter_and_strip() {
        let crc = CrcAlgorithm::crc16_modbus();
        let payload = b"STATUS_OK";
        let valid_packet = crc.append_checksum(payload);

        let mut invalid_packet = valid_packet.clone();
        let last = invalid_packet.len() - 1;
        invalid_packet[last] ^= 0xFF;

        // Packetizer with CRC filter enabled (strip_crc = false)
        let mut p = StreamPacketizer::new(FramingRule::Slip).with_crc(crc);
        let mut out = Vec::new();

        // Feed corrupted packet
        p.push_chunk(
            &StreamPacketizer::encode_slip(&invalid_packet),
            100,
            &mut out,
        );
        assert!(
            out.is_empty(),
            "Corrupted packet should be dropped by emit_frame"
        );

        // Feed valid packet
        p.push_chunk(&StreamPacketizer::encode_slip(&valid_packet), 200, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(&out[0][..], &valid_packet[..]);

        // Packetizer with strip_crc = true
        let mut p_strip = StreamPacketizer::new(FramingRule::Slip)
            .with_crc(crc)
            .with_strip_crc(true);
        let mut out_strip = Vec::new();
        p_strip.push_chunk(
            &StreamPacketizer::encode_slip(&valid_packet),
            300,
            &mut out_strip,
        );
        assert_eq!(out_strip.len(), 1);
        assert_eq!(&out_strip[0][..], payload);
    }

    #[test]
    fn test_push_chunk_events_flags_err_crc() {
        let crc = CrcAlgorithm::crc32();
        let payload = b"TELEMETRY_PACKET";
        let valid = crc.append_checksum(payload);

        let mut corrupted = valid.clone();
        corrupted[0] ^= 0x01; // flip bit

        let mut p = StreamPacketizer::new(FramingRule::Slip).with_crc(crc);
        let mut events = Vec::new();

        let mut stream = Vec::new();
        stream.extend_from_slice(&StreamPacketizer::encode_slip(&valid));
        stream.extend_from_slice(&StreamPacketizer::encode_slip(&corrupted));

        p.push_chunk_events(&stream, 500, 42, &mut events);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].flags, FrameFlags::empty());
        assert!(events[1].flags.contains(FrameFlags::ERR_CRC));
    }
}
