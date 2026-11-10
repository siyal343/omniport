//! Utility functions for hex string encoding and decoding without external dependencies

pub fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    // Work on the raw bytes so that a multi-byte UTF-8 character can never be
    // sliced at a non-char boundary (which would panic). Any non-ASCII byte is
    // simply an invalid hex digit and is reported as such.
    let clean: Vec<u8> = s
        .bytes()
        .filter(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        .collect();
    if !clean.len().is_multiple_of(2) {
        return Err(format!(
            "Hex string must have an even length, got {} characters",
            clean.len()
        ));
    }
    let nibble = |b: u8| -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    };
    let (pairs, _remainder) = clean.as_chunks::<2>();
    pairs
        .iter()
        .enumerate()
        .map(|(i, pair)| {
            let hi = nibble(pair[0]).ok_or_else(|| {
                format!("Invalid hex byte at index {}: {:?}", i * 2, pair[0] as char)
            })?;
            let lo = nibble(pair[1]).ok_or_else(|| {
                format!(
                    "Invalid hex byte at index {}: {:?}",
                    i * 2 + 1,
                    pair[1] as char
                )
            })?;
            Ok((hi << 4) | lo)
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
    Ok(haystack
        .windows(needle.len())
        .any(|w| w == needle.as_slice()))
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

    #[test]
    fn test_decode_hex_accepts_whitespace_and_mixed_case() {
        assert_eq!(
            decode_hex(" aa\tBB\r\ncC ").unwrap(),
            vec![0xAA, 0xBB, 0xCC]
        );
        assert!(decode_hex("abc").is_err());
        assert!(decode_hex("zz").is_err());
        assert!(decode_hex("0x12").is_err());
    }

    #[test]
    fn test_decode_hex_rejects_multibyte_utf8_without_panicking() {
        // Regression: 4 bytes long (even) but index 2 falls inside the 3-byte
        // '€' character. The previous char-slice implementation panicked here.
        assert!(decode_hex("a€").is_err());
        assert!(decode_hex("€a").is_err());
        assert!(decode_hex("ＡＡ").is_err());
        // Non-ASCII input must produce a normal error, never a panic.
        let err = decode_hex("a€").unwrap_err();
        assert!(err.contains("Invalid hex byte"), "unexpected error: {err}");
    }
}
