use omniport_capture::pcapng::{LINKTYPE_USER0, PcapngReader, PcapngWriter};
use omniport_capture::{FrameIndexEntry, MmapCaptureEngine};
use proptest::prelude::*;
use tempfile::tempdir;

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, cases: 50, ..ProptestConfig::default() })]

    /// FR-404: PCAPNG block encoding & decoding roundtrip across arbitrary frame lengths & timestamps
    #[test]
    fn prop_pcapng_roundtrip_fidelity(
        frames in prop::collection::vec((any::<u64>(), prop::collection::vec(any::<u8>(), 0..1024)), 1..25)
    ) {
        let mut pcapng_buf = Vec::new();
        let mut writer = PcapngWriter::with_link_type(&mut pcapng_buf, LINKTYPE_USER0);
        writer.write_header("prop_test_port").unwrap();

        for &(ts, ref payload) in &frames {
            writer.write_packet(ts, payload, 0).unwrap();
        }
        writer.flush().unwrap();

        // Invariant: Total length must be a multiple of 4 bytes (32-bit alignment)
        prop_assert_eq!(pcapng_buf.len() % 4, 0, "PCAPNG buffer length is not 4-byte aligned");

        let mut reader = PcapngReader::new(pcapng_buf.as_slice());
        let parsed = reader.read_all_packets().unwrap();

        prop_assert_eq!(parsed.len(), frames.len());
        for (i, ((exp_ts, exp_payload), actual)) in frames.iter().zip(parsed.iter()).enumerate() {
            prop_assert_eq!(actual.timestamp_ns, *exp_ts, "Timestamp mismatch at frame {}", i);
            prop_assert_eq!(&actual.payload[..], &exp_payload[..], "Payload mismatch at frame {}", i);
        }
    }

    /// FR-401 / FR-404: Dual-Mmap capture store to PCAPNG export integrity
    #[test]
    fn prop_mmap_to_pcapng_export(
        frames in prop::collection::vec(prop::collection::vec(any::<u8>(), 1..256), 1..20)
    ) {
        let dir = tempdir().unwrap();
        let mut engine = MmapCaptureEngine::create(dir.path(), 100, 65536).unwrap();

        for (i, payload) in frames.iter().enumerate() {
            let entry = FrameIndexEntry {
                timestamp_ns: 1_000_000_000 + i as u64 * 1000,
                data_offset: 0,
                payload_len: payload.len() as u32,
                endpoint_id: 1,
                flags: 0,
                pins: 0,
            };
            engine.append_frame(entry, payload);
        }

        let mut exported_bytes = Vec::new();
        let count = engine.export_pcapng(&mut exported_bytes, LINKTYPE_USER0).unwrap();
        prop_assert_eq!(count, frames.len() as u64);

        let mut reader = PcapngReader::new(exported_bytes.as_slice());
        let read_packets = reader.read_all_packets().unwrap();
        prop_assert_eq!(read_packets.len(), frames.len());

        for (i, (orig, read)) in frames.iter().zip(read_packets.iter()).enumerate() {
            prop_assert_eq!(&orig[..], &read.payload[..], "Payload mismatch at index {}", i);
        }
    }
}
