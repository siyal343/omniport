//! Cross-language contract test for the ICD §2 viewport wire format.
//!
//! `fixtures/viewport_v1.json` is generated from the ICD text alone by
//! `fixtures/generate_viewport_fixture.mjs` — it is an independent oracle, not a
//! snapshot of this implementation. This test asserts that the Rust encoder
//! reproduces it byte for byte, and that the Rust decoder can read it back.
//!
//! The TypeScript decoder is asserted against the same fixture by
//! `apps/omniport-desktop/src/ipc/viewportDecoder.contract.test.js`. If either
//! language drifts from the ICD, one of the two tests goes red.

use omniport_capture::{BATCH_HEADER_BYTES, MmapCaptureEngine, ROW_HEADER_BYTES, ViewportBatch};
use omniport_core::{FrameFlags, PinState, SerialEvent};
use serde_json::Value;
use std::path::PathBuf;

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/viewport_v1.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read fixture {}: {err}", path.display()));
    serde_json::from_str(&text).expect("viewport fixture must be valid JSON")
}

fn hex_decode(input: &str) -> Vec<u8> {
    assert!(
        input.len().is_multiple_of(2),
        "odd-length hex string: {input}"
    );
    (0..input.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&input[i..i + 2], 16).expect("hex byte"))
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn u64_field(case: &Value, key: &str) -> u64 {
    case[key]
        .as_str()
        .unwrap_or_else(|| panic!("fixture field {key} must be a decimal string"))
        .parse()
        .unwrap_or_else(|_| panic!("fixture field {key} must be a u64"))
}

/// Builds the capture store described by the case's `events` array.
fn engine_for_case(case: &Value) -> MmapCaptureEngine {
    let events = case["events"].as_array().expect("events array");
    let dir = tempfile::tempdir().expect("temp dir");
    let max_packets = events.len().max(1) as u64;
    let max_data_bytes = events
        .iter()
        .map(|event| hex_decode(event["payload_hex"].as_str().expect("payload_hex")).len())
        .sum::<usize>()
        .max(1) as u64;

    let mut engine =
        MmapCaptureEngine::create(dir.path(), max_packets, max_data_bytes).expect("capture engine");

    for event in events {
        let payload = hex_decode(event["payload_hex"].as_str().expect("payload_hex"));
        engine.append_event(&SerialEvent {
            timestamp_ns: u64_field(event, "timestamp_ns"),
            source_id: event["endpoint_id"].as_u64().expect("endpoint_id") as u16,
            flags: FrameFlags::from_bits_truncate(event["flags"].as_u64().expect("flags") as u8),
            pins: PinState::from_bits_truncate(event["pins"].as_u64().expect("pins") as u8),
            payload: bytes::Bytes::from(payload),
        });
    }

    // The engine's mmaps stay valid after the directory is removed on POSIX and
    // Windows keeps the files mapped; leaking the guard just keeps test runs
    // deterministic about cleanup order.
    std::mem::forget(dir);
    engine
}

#[test]
fn test_fixture_layout_constants_match_icd() {
    let fixture = fixture();
    assert_eq!(
        fixture["batch_header_bytes"].as_u64().unwrap() as usize,
        BATCH_HEADER_BYTES
    );
    assert_eq!(
        fixture["row_header_bytes"].as_u64().unwrap() as usize,
        ROW_HEADER_BYTES
    );
    assert_eq!(BATCH_HEADER_BYTES, 16);
    assert_eq!(ROW_HEADER_BYTES, 24);
    assert_eq!(fixture["endianness"].as_str().unwrap(), "little");
}

#[test]
fn test_rust_encoder_and_decoder_reproduce_frozen_contract() {
    let fixture = fixture();
    let cases = fixture["cases"].as_array().expect("cases array");
    assert!(!cases.is_empty(), "fixture must contain cases");

    for case in cases {
        let id = case["id"].as_str().expect("case id");
        let engine = engine_for_case(case);
        let start_row = u64_field(case, "start_row");
        let row_count = case["row_count"].as_u64().expect("row_count") as usize;

        let batch = engine.read_viewport_batch(start_row, row_count);

        // 1. Header/row counts agree with the fixture.
        assert_eq!(
            batch.total_packets(),
            u64_field(case, "total_packets"),
            "case {id}: total_packets"
        );
        let expected_rows = case["rows"].as_array().expect("rows array");
        assert_eq!(
            batch.returned_rows(),
            expected_rows.len(),
            "case {id}: returned_rows"
        );

        // 2. The encoder reproduces the frozen wire bytes exactly.
        let wire = batch.encode_to_vec();
        let wire_hex = hex_encode(&wire);
        assert_eq!(
            wire_hex,
            case["wire_hex"].as_str().expect("wire_hex"),
            "case {id}: encoded bytes diverge from the ICD fixture (wire_len={})",
            wire.len()
        );
        assert_eq!(
            wire.len(),
            case["wire_len"].as_u64().unwrap() as usize,
            "case {id}: wire_len"
        );

        // 3. Every row field crosses the seam intact.
        for (index, (actual, expected)) in batch.rows().iter().zip(expected_rows).enumerate() {
            assert_eq!(
                actual.timestamp_ns,
                u64_field(expected, "timestamp_ns"),
                "case {id} row {index}: timestamp_ns"
            );
            assert_eq!(
                actual.data_offset,
                u64_field(expected, "data_offset"),
                "case {id} row {index}: data_offset"
            );
            assert_eq!(
                actual.endpoint_id,
                expected["endpoint_id"].as_u64().unwrap() as u16,
                "case {id} row {index}: endpoint_id"
            );
            assert_eq!(
                actual.flags.bits(),
                expected["flags"].as_u64().unwrap() as u8,
                "case {id} row {index}: flags"
            );
            assert_eq!(
                actual.pins.bits(),
                expected["pins"].as_u64().unwrap() as u8,
                "case {id} row {index}: pins"
            );
            assert_eq!(
                hex_encode(actual.payload),
                expected["payload_hex"].as_str().expect("payload_hex"),
                "case {id} row {index}: payload"
            );
        }

        // 4. The decoder walks the same wire bytes back to the same rows.
        let decoded = ViewportBatch::decode(&wire)
            .unwrap_or_else(|err| panic!("case {id}: fixture wire failed to decode: {err}"));
        assert_eq!(
            decoded.total_packets(),
            batch.total_packets(),
            "case {id}: decode total"
        );
        assert_eq!(
            decoded.rows().len(),
            batch.rows().len(),
            "case {id}: decode rows"
        );
        for (index, (decoded_row, encoded_row)) in
            decoded.rows().iter().zip(batch.rows()).enumerate()
        {
            assert_eq!(
                decoded_row.timestamp_ns, encoded_row.timestamp_ns,
                "case {id} row {index}: decoded timestamp"
            );
            assert_eq!(decoded_row.endpoint_id, encoded_row.endpoint_id);
            assert_eq!(decoded_row.flags, encoded_row.flags);
            assert_eq!(decoded_row.pins, encoded_row.pins);
            assert_eq!(decoded_row.payload, encoded_row.payload);
        }
    }
}

#[test]
fn test_decoder_rejects_truncated_fixture_wire() {
    let fixture = fixture();
    let cases = fixture["cases"].as_array().expect("cases array");
    for case in cases {
        let wire = hex_decode(case["wire_hex"].as_str().expect("wire_hex"));
        if wire.len() <= BATCH_HEADER_BYTES {
            continue;
        }
        // Dropping the last byte must be rejected, never panic.
        let truncated = &wire[..wire.len() - 1];
        assert!(
            ViewportBatch::decode(truncated).is_err(),
            "case {}: truncated wire must be rejected",
            case["id"]
        );
    }
}
