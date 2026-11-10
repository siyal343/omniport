//! ICD Section 2 viewport wire format — the single owner of the layout.
//!
//! The dual-mmap capture store exposes a window of rows to the GUI through a
//! binary batch whose layout is specified in `internal_docs/ICD.md` §2:
//!
//! ```text
//! [total_packets: u64][returned_rows: u32][reserved: u32]
//! repeated returned_rows times:
//!     [timestamp_ns: u64][data_offset: u64][payload_len: u32]
//!     [endpoint_id: u16][flags: u8][pins: u8][payload: payload_len bytes]
//! ```
//!
//! Everything about the layout — header size, row stride, reserved word,
//! endianness, bounds checks, request clamping and the payload copy — lives
//! behind this module's small interface so the Tauri command, the Wireshark
//! extcap streamer and the TypeScript decoder cannot drift apart by hand.
//!
//! The layout is frozen by `fixtures/viewport_v1.json`, an independent oracle
//! asserted from both Rust (`tests/viewport_contract.rs`) and TypeScript
//! (`src/ipc/viewportDecoder.contract.test.js`).

use crate::{FrameIndexEntry, MmapCaptureEngine};
use omniport_core::{EndpointId, FrameFlags, PinState};
use std::sync::atomic::Ordering;
use thiserror::Error;

/// Upper bound on how many rows a single viewport request may return.
///
/// The GUI only ever asks for one screenful plus overscan (tens of rows); the
/// clamp keeps an out-of-range `row_count` from driving a huge allocation.
pub const MAX_VIEWPORT_ROWS: usize = 4096;

/// Fixed 16-byte batch header: `total_packets`, `returned_rows`, reserved word.
pub const BATCH_HEADER_BYTES: usize = 16;
/// Fixed 24-byte row header, identical to the on-disk [`FrameIndexEntry`].
pub const ROW_HEADER_BYTES: usize = std::mem::size_of::<FrameIndexEntry>();

const _: () = assert!(ROW_HEADER_BYTES == 24);

/// One decoded row of a viewport batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportRow<'a> {
    pub timestamp_ns: u64,
    pub data_offset: u64,
    pub endpoint_id: EndpointId,
    pub flags: FrameFlags,
    pub pins: PinState,
    pub payload: &'a [u8],
}

/// Error returned when a byte slice does not contain a valid ICD §2 batch.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum ViewportDecodeError {
    #[error("viewport batch shorter than the {BATCH_HEADER_BYTES}-byte header")]
    TruncatedHeader,
    #[error("viewport batch truncated inside row {row_index}")]
    TruncatedRow { row_index: usize },
}

/// A contiguous window of capture rows plus the session row count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewportBatch<'a> {
    total_packets: u64,
    rows: Vec<ViewportRow<'a>>,
}

impl<'a> ViewportBatch<'a> {
    /// Builds a batch from its parts. Row order is preserved on the wire.
    pub fn new(total_packets: u64, rows: Vec<ViewportRow<'a>>) -> Self {
        Self {
            total_packets,
            rows,
        }
    }

    /// Total number of frames in the capture session (not clamped by the request).
    pub fn total_packets(&self) -> u64 {
        self.total_packets
    }

    /// Rows contained in this window, in row order.
    pub fn rows(&self) -> &[ViewportRow<'a>] {
        &self.rows
    }

    /// Number of rows in this window (the wire's `returned_rows`).
    pub fn returned_rows(&self) -> usize {
        self.rows.len()
    }

    /// Byte length of this batch once encoded.
    pub fn encoded_len(&self) -> usize {
        BATCH_HEADER_BYTES
            + self
                .rows
                .iter()
                .map(|row| ROW_HEADER_BYTES + row.payload.len())
                .sum::<usize>()
    }

    /// Serializes the batch into `out`, appending to whatever is already there.
    pub fn encode(&self, out: &mut Vec<u8>) {
        out.reserve(self.encoded_len());

        out.extend_from_slice(&self.total_packets.to_le_bytes());
        out.extend_from_slice(&(self.rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // reserved, always zero

        for row in &self.rows {
            out.extend_from_slice(&row.timestamp_ns.to_le_bytes());
            out.extend_from_slice(&row.data_offset.to_le_bytes());
            out.extend_from_slice(&(row.payload.len() as u32).to_le_bytes());
            out.extend_from_slice(&row.endpoint_id.to_le_bytes());
            out.push(row.flags.bits());
            out.push(row.pins.bits());
            out.extend_from_slice(row.payload);
        }
    }

    /// Convenience wrapper around [`ViewportBatch::encode`].
    pub fn encode_to_vec(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.encoded_len());
        self.encode(&mut out);
        out
    }

    /// Parses an ICD §2 batch out of `bytes`.
    ///
    /// Payloads are borrowed zero-copy from `bytes`; the caller keeps ownership.
    pub fn decode(bytes: &'a [u8]) -> Result<Self, ViewportDecodeError> {
        if bytes.len() < BATCH_HEADER_BYTES {
            return Err(ViewportDecodeError::TruncatedHeader);
        }

        let total_packets = u64::from_le_bytes(
            bytes[0..8]
                .try_into()
                .map_err(|_| ViewportDecodeError::TruncatedHeader)?,
        );
        let returned_rows = u32::from_le_bytes(
            bytes[8..12]
                .try_into()
                .map_err(|_| ViewportDecodeError::TruncatedHeader)?,
        ) as usize;

        let mut rows = Vec::with_capacity(returned_rows.min(MAX_VIEWPORT_ROWS));
        let mut offset = BATCH_HEADER_BYTES;

        for row_index in 0..returned_rows {
            let header = bytes
                .get(offset..offset + ROW_HEADER_BYTES)
                .ok_or(ViewportDecodeError::TruncatedRow { row_index })?;

            let timestamp_ns = u64::from_le_bytes(header[0..8].try_into().unwrap_or_default());
            let data_offset = u64::from_le_bytes(header[8..16].try_into().unwrap_or_default());
            let payload_len = u32::from_le_bytes(header[16..20].try_into().unwrap_or_default());
            let endpoint_id = u16::from_le_bytes(header[20..22].try_into().unwrap_or_default());
            let flags = FrameFlags::from_bits_truncate(header[22]);
            let pins = PinState::from_bits_truncate(header[23]);

            let payload_start = offset + ROW_HEADER_BYTES;
            let payload_end = payload_start
                .checked_add(payload_len as usize)
                .ok_or(ViewportDecodeError::TruncatedRow { row_index })?;
            let payload = bytes
                .get(payload_start..payload_end)
                .ok_or(ViewportDecodeError::TruncatedRow { row_index })?;

            rows.push(ViewportRow {
                timestamp_ns,
                data_offset,
                endpoint_id,
                flags,
                pins,
                payload,
            });
            offset = payload_end;
        }

        Ok(Self {
            total_packets,
            rows,
        })
    }
}

impl MmapCaptureEngine {
    /// Reads a clamped window of the capture store as a [`ViewportBatch`].
    ///
    /// `start_row` past the end yields an empty batch that still reports
    /// `total_packets`. `row_count` is clamped to [`MAX_VIEWPORT_ROWS`] and to
    /// the number of rows that actually exist.
    pub fn read_viewport_batch(&self, start_row: u64, row_count: usize) -> ViewportBatch<'_> {
        let total = self.packet_count.load(Ordering::Acquire);
        if start_row >= total {
            return ViewportBatch::new(total, Vec::new());
        }

        let bounded = (row_count.min(MAX_VIEWPORT_ROWS) as u64).min(total - start_row);
        let end_row = start_row.saturating_add(bounded);

        let mut rows = Vec::with_capacity(bounded as usize);
        for row in start_row..end_row {
            let offset = (row as usize) * ROW_HEADER_BYTES;
            let Some(entry_bytes) = self.idx_mmap.get(offset..offset + ROW_HEADER_BYTES) else {
                break;
            };
            let entry: &FrameIndexEntry = bytemuck::from_bytes(entry_bytes);

            let payload_start = entry.data_offset as usize;
            let payload_end = payload_start.saturating_add(entry.payload_len as usize);
            let payload = self.dat_mmap.get(payload_start..payload_end).unwrap_or(&[]);

            rows.push(ViewportRow {
                timestamp_ns: entry.timestamp_ns,
                data_offset: entry.data_offset,
                endpoint_id: entry.endpoint_id,
                flags: FrameFlags::from_bits_truncate(entry.flags),
                pins: PinState::from_bits_truncate(entry.pins),
                payload,
            });
        }

        ViewportBatch::new(total, rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MmapCaptureEngine;
    use omniport_core::SerialEvent;
    use tempfile::tempdir;

    fn engine_with_events(events: &[(u64, EndpointId, u8, u8, &[u8])]) -> MmapCaptureEngine {
        let dir = tempdir().unwrap();
        let max_packets = events.len().max(1) as u64;
        let data_bytes = events
            .iter()
            .map(|(_, _, _, _, payload)| payload.len() as u64)
            .sum::<u64>()
            .max(1);
        let mut engine = MmapCaptureEngine::create(dir.path(), max_packets, data_bytes).unwrap();
        for (timestamp_ns, endpoint_id, flags, pins, payload) in events {
            engine.append_event(&SerialEvent {
                timestamp_ns: *timestamp_ns,
                source_id: *endpoint_id,
                flags: FrameFlags::from_bits_truncate(*flags),
                pins: PinState::from_bits_truncate(*pins),
                payload: bytes::Bytes::copy_from_slice(payload),
            });
        }
        // The engine owns file-backed mmaps; keep the temp dir alive by leaking it.
        std::mem::forget(dir);
        engine
    }

    #[test]
    fn test_batch_header_layout_is_frozen() {
        let engine = engine_with_events(&[(7, 1, 0x01, 0x05, b"AB")]);
        let encoded = engine.read_viewport_batch(0, 10).encode_to_vec();

        assert_eq!(BATCH_HEADER_BYTES, 16);
        assert_eq!(ROW_HEADER_BYTES, 24);
        assert_eq!(encoded.len(), BATCH_HEADER_BYTES + ROW_HEADER_BYTES + 2);
        assert_eq!(u64::from_le_bytes(encoded[0..8].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(encoded[8..12].try_into().unwrap()), 1);
        assert_eq!(
            &encoded[12..16],
            &[0, 0, 0, 0],
            "reserved word must be zero"
        );
        assert_eq!(u64::from_le_bytes(encoded[16..24].try_into().unwrap()), 7);
        assert_eq!(
            u32::from_le_bytes(encoded[32..36].try_into().unwrap()),
            2,
            "payload_len at row offset 16"
        );
        assert_eq!(
            u16::from_le_bytes(encoded[36..38].try_into().unwrap()),
            1,
            "endpoint_id at row offset 20"
        );
        assert_eq!(encoded[38], 0x01, "flags at row offset 22");
        assert_eq!(encoded[39], 0x05, "pins at row offset 23");
        assert_eq!(&encoded[40..42], b"AB");
    }

    #[test]
    fn test_roundtrip_preserves_every_field() {
        let engine = engine_with_events(&[
            (0, 1, 0x01, 0x05, b"first"),
            (u64::MAX, u16::MAX, 0x40, 0xff, b""),
            (3, 7, 0x30, 0x38, &[0u8; 33]),
        ]);
        let batch = engine.read_viewport_batch(1, 2);
        assert_eq!(batch.total_packets(), 3);
        assert_eq!(batch.returned_rows(), 2);
        assert_eq!(batch.rows()[0].timestamp_ns, u64::MAX);
        assert_eq!(batch.rows()[0].endpoint_id, u16::MAX);
        assert_eq!(batch.rows()[0].flags, FrameFlags::from_bits_truncate(0x40));
        assert_eq!(batch.rows()[0].pins, PinState::from_bits_truncate(0xff));
        assert!(batch.rows()[0].payload.is_empty());
        assert_eq!(batch.rows()[1].payload.len(), 33);

        let wire = batch.encode_to_vec();
        let decoded = ViewportBatch::decode(&wire).unwrap();
        assert_eq!(decoded.total_packets(), batch.total_packets());
        assert_eq!(decoded.rows().len(), batch.rows().len());
        for (a, b) in decoded.rows().iter().zip(batch.rows()) {
            assert_eq!(a.timestamp_ns, b.timestamp_ns);
            assert_eq!(a.data_offset, b.data_offset);
            assert_eq!(a.endpoint_id, b.endpoint_id);
            assert_eq!(a.flags, b.flags);
            assert_eq!(a.pins, b.pins);
            assert_eq!(a.payload, b.payload);
        }
    }

    #[test]
    fn test_out_of_range_start_reports_total_with_no_rows() {
        let engine = engine_with_events(&[(1, 1, 0, 0, b"a")]);
        let batch = engine.read_viewport_batch(u64::MAX, MAX_VIEWPORT_ROWS);
        assert_eq!(batch.total_packets(), 1);
        assert_eq!(batch.returned_rows(), 0);
        assert_eq!(batch.encode_to_vec().len(), BATCH_HEADER_BYTES);
    }

    #[test]
    fn test_row_count_is_clamped_to_store_and_to_max() {
        let rows: Vec<_> = (0..(MAX_VIEWPORT_ROWS as u64 + 10))
            .map(|i| (i, 1u16, 0u8, 0u8, &b"x"[..]))
            .collect();
        let engine = engine_with_events(&rows);
        let batch = engine.read_viewport_batch(0, usize::MAX);
        assert_eq!(batch.returned_rows(), MAX_VIEWPORT_ROWS);

        let small = engine.read_viewport_batch(0, 3);
        assert_eq!(small.returned_rows(), 3);
    }

    #[test]
    fn test_decode_rejects_truncated_batches() {
        assert_eq!(
            ViewportBatch::decode(&[0u8; 15]),
            Err(ViewportDecodeError::TruncatedHeader)
        );

        let engine = engine_with_events(&[(1, 1, 0, 0, b"payload")]);
        let wire = engine.read_viewport_batch(0, 1).encode_to_vec();

        // Truncated inside the row header.
        assert!(matches!(
            ViewportBatch::decode(&wire[..wire.len() - 10]),
            Err(ViewportDecodeError::TruncatedRow { row_index: 0 })
        ));
        // Header present, payload missing: declared one row, none complete.
        let mut overcount = wire[..BATCH_HEADER_BYTES + ROW_HEADER_BYTES].to_vec();
        overcount[8..12].copy_from_slice(&1u32.to_le_bytes());
        assert!(matches!(
            ViewportBatch::decode(&overcount),
            Err(ViewportDecodeError::TruncatedRow { row_index: 0 })
        ));
    }
}
