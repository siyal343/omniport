//! Utility functions for hex string encoding and decoding without external dependencies

pub fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    let clean = s.trim().replace([' ', '\t', '\r', '\n'], "");
    if !clean.len().is_multiple_of(2) {
        return Err(format!(
            "Hex string must have an even length, got {} characters",
            clean.len()
        ));
    }
    (0..clean.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&clean[i..i + 2], 16)
                .map_err(|e| format!("Invalid hex byte at index {i}: {e}"))
        })
        .collect()
}

pub fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

pub fn hex_matches(haystack: &[u8], needle_hex: &str) -> Result<bool, String> {
    let needle = decode_hex(needle_hex)?;
    if needle.is_empty() {
        return Ok(true);
    }
    Ok(haystack.windows(needle.len()).any(|w| w == needle.as_slice()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_roundtrip() {
        let original = vec![0x01, 0x03, 0x00, 0x00, 0x00, 0x02, 0xC4, 0x0B];
        let hex_str = encode_hex(&original);
        assert_eq!(hex_str, "010300000002C40B");
        let decoded = decode_hex(&hex_str).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_hex_matches() {
        let data = [0xAA, 0xBB, 0xCC, 0xDD];
        assert!(hex_matches(&data, "BBCC").unwrap());
        assert!(!hex_matches(&data, "FFEE").unwrap());
    }
}
