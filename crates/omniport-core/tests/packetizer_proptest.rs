use bytes::Bytes;
use omniport_core::crc::CrcAlgorithm;
use omniport_core::framing::{FramingRule, StreamPacketizer};
use proptest::prelude::*;
use std::time::Duration;

/// Slices a contiguous byte stream into arbitrary chunk sizes defined by chunk_sizes.
fn chunkify(stream: &[u8], chunk_sizes: &[usize]) -> Vec<Vec<u8>> {
    let mut chunks = Vec::new();
    let mut offset = 0;
    let mut size_idx = 0;

    while offset < stream.len() {
        let size = if chunk_sizes.is_empty() {
            1
        } else {
            (chunk_sizes[size_idx % chunk_sizes.len()] % 32) + 1
        };
        size_idx += 1;
        let end = (offset + size).min(stream.len());
        chunks.push(stream[offset..end].to_vec());
        offset = end;
    }
    chunks
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, cases: 100, ..ProptestConfig::default() })]

    /// FR-201 / VV_RTM: Verify 100% SLIP frame recovery across arbitrary byte fragmentation
    #[test]
    fn prop_slip_framing_recovery(
        frames in prop::collection::vec(prop::collection::vec(any::<u8>(), 1..128), 1..20),
        chunk_sizes in prop::collection::vec(1usize..64, 1..30)
    ) {
        let mut stream = Vec::new();
        for frame in &frames {
            let encoded = StreamPacketizer::encode_slip(frame);
            stream.extend_from_slice(&encoded);
        }

        let chunks = chunkify(&stream, &chunk_sizes);
        let mut packetizer = StreamPacketizer::new(FramingRule::Slip);
        let mut recovered = Vec::new();

        let mut now_ns = 1_000_000u64;
        for chunk in chunks {
            packetizer.push_chunk(&chunk, now_ns, &mut recovered);
            now_ns += 100;
        }

        prop_assert_eq!(recovered.len(), frames.len(), "Frame count mismatch");
        for (i, (rec, orig)) in recovered.iter().zip(frames.iter()).enumerate() {
            prop_assert_eq!(&rec[..], &orig[..], "Frame {} payload mismatch", i);
        }
    }

    /// FR-201 / VV_RTM: Verify 100% COBS frame recovery across arbitrary byte fragmentation
    #[test]
    fn prop_cobs_framing_recovery(
        frames in prop::collection::vec(prop::collection::vec(any::<u8>(), 1..128), 1..20),
        chunk_sizes in prop::collection::vec(1usize..64, 1..30)
    ) {
        let mut stream = Vec::new();
        for frame in &frames {
            let encoded = StreamPacketizer::encode_cobs(frame);
            stream.extend_from_slice(&encoded);
        }

        let chunks = chunkify(&stream, &chunk_sizes);
        let mut packetizer = StreamPacketizer::new(FramingRule::Cobs);
        let mut recovered = Vec::new();

        let mut now_ns = 1_000_000u64;
        for chunk in chunks {
            packetizer.push_chunk(&chunk, now_ns, &mut recovered);
            now_ns += 100;
        }

        prop_assert_eq!(recovered.len(), frames.len(), "COBS frame count mismatch");
        for (i, (rec, orig)) in recovered.iter().zip(frames.iter()).enumerate() {
            prop_assert_eq!(&rec[..], &orig[..], "COBS frame {} payload mismatch", i);
        }
    }

    /// FR-201: Verify Length-Field framing with sync word across arbitrary byte fragmentation
    #[test]
    fn prop_length_field_framing_recovery(
        payloads in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..64), 1..15),
        chunk_sizes in prop::collection::vec(1usize..32, 1..20)
    ) {
        let sync_word = vec![0xEB, 0x90];
        let mut stream = Vec::new();
        let mut expected_frames = Vec::new();

        for payload in &payloads {
            let mut frame = Vec::new();
            frame.extend_from_slice(&sync_word);
            let len = payload.len() as u16;
            frame.extend_from_slice(&len.to_le_bytes()); // 2 bytes len_offset = 2
            frame.extend_from_slice(payload);
            expected_frames.push(frame.clone());
            stream.extend_from_slice(&frame);
        }

        let rule = FramingRule::LengthField {
            sync_word: sync_word.clone(),
            len_offset: 2,
            len_bytes: 2,
            little_endian: true,
            header_overhead: 4, // 2B sync + 2B len
        };

        let chunks = chunkify(&stream, &chunk_sizes);
        let mut packetizer = StreamPacketizer::new(rule);
        let mut recovered = Vec::new();

        let mut now_ns = 1_000_000u64;
        for chunk in chunks {
            packetizer.push_chunk(&chunk, now_ns, &mut recovered);
            now_ns += 100;
        }

        prop_assert_eq!(recovered.len(), expected_frames.len());
        for (rec, exp) in recovered.iter().zip(expected_frames.iter()) {
            prop_assert_eq!(&rec[..], &exp[..]);
        }
    }

    /// FR-201 / FR-302: CRC8/16/32 mathematical integrity property under bit mutations
    #[test]
    fn prop_crc_validation_and_mutation(
        payload in prop::collection::vec(any::<u8>(), 1..128),
        bit_to_flip in 0usize..1024,
        algo_choice in 0u8..3
    ) {
        let algo = match algo_choice {
            0 => CrcAlgorithm::crc8(),
            1 => CrcAlgorithm::crc16_modbus(),
            _ => CrcAlgorithm::crc32(),
        };

        let frame = algo.append_checksum(&payload);
        prop_assert!(algo.validate(&frame), "Valid checksum must pass validation");

        let stripped = algo.strip_crc(&frame);
        prop_assert_eq!(stripped, Some(Bytes::copy_from_slice(&payload)));

        // Inject 1-bit corruption
        let mut corrupted = frame.clone();
        let byte_idx = (bit_to_flip / 8) % corrupted.len();
        let bit_mask = 1u8 << (bit_to_flip % 8);
        corrupted[byte_idx] ^= bit_mask;

        prop_assert!(!algo.validate(&corrupted), "Corrupted frame must fail CRC validation");
    }

    /// FR-201: Inter-Character Idle-Gap boundary property
    #[test]
    fn prop_idle_gap_timer_boundary(
        chunks in prop::collection::vec(prop::collection::vec(any::<u8>(), 1..32), 2..10),
        gap_ms in 5u64..20
    ) {
        let gap = Duration::from_millis(gap_ms);
        let gap_ns = gap.as_nanos() as u64;
        let mut packetizer = StreamPacketizer::new(FramingRule::IdleGap(gap));
        let mut out = Vec::new();

        let mut current_ns = 1_000_000_000u64;
        let mut total_expected_bytes = Vec::new();

        // Feed all chunks within the gap (< gap_ns)
        for chunk in &chunks {
            total_expected_bytes.extend_from_slice(chunk);
            packetizer.push_chunk(chunk, current_ns, &mut out);
            current_ns += gap_ns / 2; // within gap
        }

        // Before gap expires, no frame should be emitted
        prop_assert!(out.is_empty(), "No frame should be emitted before gap timeout");

        // Advance time past gap
        current_ns += gap_ns + 100;
        packetizer.check_timeout(current_ns, &mut out);

        prop_assert_eq!(out.len(), 1, "Exactly one frame must be emitted after gap timeout");
        prop_assert_eq!(&out[0][..], &total_expected_bytes[..]);
    }
}
