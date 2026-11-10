use omniport_dissect::{Endian, PacketSchema, PrimitiveType};
use proptest::prelude::*;

#[test]
fn test_pragma_pack_syntax() {
    let c_code = r#"
        #pragma pack(push, 1)
        struct GpsFix {
            uint8_t fix_type;
            int32_t lat;
            int32_t lon;
            uint16_t alt_m;
            uint8_t satellites: 4;
            uint8_t reserved: 4;
        };
        #pragma pack(pop)
    "#;

    let schema = PacketSchema::from_c_struct(c_code, Endian::Little).unwrap();
    assert_eq!(schema.name, "GpsFix");
    // fix_type: 1B
    // lat: 4B
    // lon: 4B
    // alt_m: 2B
    // satellites (4b) + reserved (4b): 1B
    // Total = 1 + 4 + 4 + 2 + 1 = 12 bytes
    assert_eq!(schema.total_size, 12);

    let mut payload = Vec::new();
    payload.push(3); // fix_type = 3
    payload.extend_from_slice(&37774900i32.to_le_bytes()); // lat
    payload.extend_from_slice(&(-122419400i32).to_le_bytes()); // lon
    payload.extend_from_slice(&450u16.to_le_bytes()); // alt_m
    payload.push(0x0C); // satellites: 12 (0x0C), reserved: 0

    let decoded = schema.dissect_frame(&payload).unwrap();
    assert_eq!(decoded.len(), 6);
    assert_eq!(decoded[0].name, "fix_type");
    assert_eq!(decoded[0].value_f64, 3.0);
    assert_eq!(decoded[1].name, "lat");
    assert_eq!(decoded[1].value_f64, 37774900.0);
    assert_eq!(decoded[2].name, "lon");
    assert_eq!(decoded[2].value_f64, -122419400.0);
    assert_eq!(decoded[3].name, "alt_m");
    assert_eq!(decoded[3].value_f64, 450.0);
    assert_eq!(decoded[4].name, "satellites");
    assert_eq!(decoded[4].value_f64, 12.0);
    assert_eq!(decoded[5].name, "reserved");
    assert_eq!(decoded[5].value_f64, 0.0);
}

#[test]
fn test_typedef_struct_syntax() {
    let c_code = r#"
        typedef struct {
            uint16_t header;
            uint8_t payload_len;
        } PacketHeader_t;
    "#;

    let schema = PacketSchema::from_c_struct(c_code, Endian::Big).unwrap();
    assert_eq!(schema.name, "PacketHeader_t");
    assert_eq!(schema.total_size, 3);

    let payload = [0x12, 0x34, 0x42];
    let decoded = schema.dissect_frame(&payload).unwrap();
    assert_eq!(decoded[0].value_f64, 0x1234 as f64);
    assert_eq!(decoded[1].value_f64, 66.0);
}

#[test]
fn test_signed_bitfield_sign_extension() {
    let c_code = r#"
        struct __attribute__((packed)) SignedBitfields {
            int8_t signed_3bit: 3;
            int8_t signed_5bit: 5;
        };
    "#;

    let schema = PacketSchema::from_c_struct(c_code, Endian::Little).unwrap();
    assert_eq!(schema.total_size, 1);

    // signed_3bit = -1 (0b111 = 7)
    // signed_5bit = -4 (0b11100 = 28)
    // byte = 7 | (28 << 3) = 7 | 224 = 231 (0xE7)
    let payload = [0xE7];
    let decoded = schema.dissect_frame(&payload).unwrap();

    assert_eq!(decoded[0].name, "signed_3bit");
    assert_eq!(decoded[0].value_f64, -1.0);

    assert_eq!(decoded[1].name, "signed_5bit");
    assert_eq!(decoded[1].value_f64, -4.0);
}

#[test]
fn test_multi_byte_bitfields() {
    let c_code = r#"
        struct __attribute__((packed)) Container16 {
            uint16_t angle_raw: 12; // 0..4095
            uint16_t status_flags: 4; // 0..15
        };
    "#;

    let schema = PacketSchema::from_c_struct(c_code, Endian::Little).unwrap();
    assert_eq!(schema.total_size, 2);

    let angle = 3000u16; // 0b1011_1011_1000
    let flags = 0xAu16; // 0b1010
    let combined = angle | (flags << 12);
    let payload = combined.to_le_bytes();

    let decoded = schema.dissect_frame(&payload).unwrap();
    assert_eq!(decoded[0].name, "angle_raw");
    assert_eq!(decoded[0].value_f64, 3000.0);

    assert_eq!(decoded[1].name, "status_flags");
    assert_eq!(decoded[1].value_f64, 10.0);
}

#[test]
fn test_mixed_bitfield_container_types_layout() {
    let c_code = r#"
        struct __attribute__((packed)) MixedContainers {
            uint8_t  a: 4;
            uint16_t b: 4;
        };
    "#;
    let schema = PacketSchema::from_c_struct(c_code, Endian::Little).unwrap();

    // `a` owns one u8 container and `b` a full u16 container starting after it,
    // so the packed struct is 3 bytes (matching the C compiler's layout).
    assert_eq!(schema.total_size, 3);

    // A frame matching the old (undercounted) total size must be rejected, not panic
    // on a bitfield container slice that runs past the end of the payload.
    assert!(schema.dissect_frame(&[0u8; 2]).is_none());

    let decoded = schema.dissect_frame(&[0x0A, 0x34, 0x12]).unwrap();
    assert_eq!(decoded[0].name, "a");
    assert_eq!(decoded[0].value_f64, 10.0);
    assert_eq!(decoded[1].name, "b");
    assert_eq!(decoded[1].value_f64, 4.0);
}

#[test]
fn test_bitfield_container_does_not_share_storage_unit_with_wider_type() {
    let c_code = r#"
        struct __attribute__((packed)) Overflow16 {
            uint16_t first: 12;
            uint16_t second: 8;
            uint16_t third: 12;
        };
    "#;
    let schema = PacketSchema::from_c_struct(c_code, Endian::Little).unwrap();
    // first occupies bytes 0-1 (12 bits used), second needs 8 bits and cannot share
    // that unit -> bytes 2-3, third cannot share the remaining 8 bits of second's
    // unit -> bytes 4-5. Total 6 bytes.
    assert_eq!(schema.total_size, 6);
    assert!(schema.dissect_frame(&[0u8; 5]).is_none());
    assert!(schema.dissect_frame(&[0u8; 6]).is_some());
}

#[test]
fn test_trailing_partial_bitfield_container_occupies_full_storage_unit() {
    let schema = PacketSchema::from_c_struct(
        "struct __attribute__((packed)) Partial { uint16_t a: 4; };",
        Endian::Little,
    )
    .unwrap();
    // A u16 bitfield allocates a full 16-bit storage unit even when only 4 bits are used.
    assert_eq!(schema.total_size, 2);
}

#[test]
fn test_signed_64bit_bitfield_sign_extension() {
    let schema = PacketSchema::from_c_struct(
        "struct __attribute__((packed)) S64 { int64_t value: 64; };",
        Endian::Little,
    )
    .unwrap();
    assert_eq!(schema.total_size, 8);
    let decoded = schema.dissect_frame(&(-1i64).to_le_bytes()).unwrap();
    assert_eq!(decoded[0].name, "value");
    assert_eq!(decoded[0].value_f64, -1.0);
}

#[test]
fn test_huge_array_count_is_error_not_panic() {
    // 2^62 elements of u32 overflows usize when computing the struct size; the parser
    // must report an error instead of panicking on the multiply.
    let result = PacketSchema::from_c_struct(
        "struct Huge { uint32_t values[4611686018427387904]; };",
        Endian::Little,
    );
    assert!(result.is_err());
}

#[test]
fn test_long_long_and_unsigned_long_long_types() {
    // The parser documents support for the `long long` aliases; consuming only one
    // `long` would treat the second keyword as the field name and fail (or mis-layout).
    let schema = PacketSchema::from_c_struct(
        "struct Aliases { unsigned long long u64_value; long long i64_value; unsigned short u16_value; long i32_value; };",
        Endian::Little,
    )
    .unwrap();
    assert_eq!(schema.fields.len(), 4);
    assert_eq!(schema.fields[0].ty, PrimitiveType::U64);
    assert_eq!(schema.fields[1].ty, PrimitiveType::I64);
    assert_eq!(schema.fields[2].ty, PrimitiveType::U16);
    assert_eq!(schema.fields[3].ty, PrimitiveType::I32);
    assert_eq!(schema.total_size, 8 + 8 + 2 + 4);
}

#[test]
fn test_bitfield_then_scalar_reserves_full_storage_unit() {
    // Reference layout from gcc (packed, x86_64-mingw32):
    //   struct { uint16_t a:4; uint8_t b; } -> sizeof 3, `b` at byte offset 2.
    // The previous implementation advanced only by the used bits (byte 1) and undercounted
    // the struct size, so `b` was decoded from the wrong byte.
    let schema = PacketSchema::from_c_struct(
        "struct __attribute__((packed)) BitThenScalar { uint16_t a: 4; uint8_t b; };",
        Endian::Little,
    )
    .unwrap();
    assert_eq!(schema.total_size, 3);
    assert_eq!(schema.fields[1].offset, 2);
    assert!(schema.dissect_frame(&[0u8; 2]).is_none());

    let decoded = schema.dissect_frame(&[0x0A, 0x00, 0x5A]).unwrap();
    assert_eq!(decoded[0].value_f64, 10.0);
    assert_eq!(decoded[1].value_f64, 0x5A as f64);
}

#[test]
fn test_float_bitfield_is_rejected() {
    // A C struct cannot declare a float bitfield; accepting it would build a schema that
    // silently fails to dissect every frame.
    let result = PacketSchema::from_c_struct("struct Bad { float ratio: 3; };", Endian::Little);
    assert!(result.is_err());
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, cases: 100, ..ProptestConfig::default() })]

    /// FR-402 / VV_RTM: Fuzz testing C-struct parser dissection across arbitrary random payloads
    #[test]
    fn prop_c_struct_fuzz_dissection(
        sync in any::<u16>(),
        temp in any::<i16>(),
        seq in any::<u32>(),
        ax in any::<f32>(),
        ay in any::<f32>(),
        az in any::<f32>(),
        flag_a in 0u8..8,   // 3 bits
        flag_b in 0u8..2,   // 1 bit
        flag_c in 0u8..16   // 4 bits
    ) {
        let c_code = r#"
            struct __attribute__((packed)) AvionicsTelemetry {
                uint16_t sync_word;
                int16_t temperature; // scale: 0.1
                uint32_t sequence;
                float accel[3];
                uint8_t flag_a: 3;
                uint8_t flag_b: 1;
                uint8_t flag_c: 4;
            };
        "#;

        let schema = PacketSchema::from_c_struct(c_code, Endian::Little).unwrap();
        prop_assert_eq!(schema.total_size, 21);

        let mut payload = Vec::with_capacity(21);
        payload.extend_from_slice(&sync.to_le_bytes());
        payload.extend_from_slice(&temp.to_le_bytes());
        payload.extend_from_slice(&seq.to_le_bytes());
        payload.extend_from_slice(&ax.to_le_bytes());
        payload.extend_from_slice(&ay.to_le_bytes());
        payload.extend_from_slice(&az.to_le_bytes());

        let bitfield_byte = flag_a | (flag_b << 3) | (flag_c << 4);
        payload.push(bitfield_byte);

        let decoded = schema.dissect_frame(&payload);
        prop_assert!(decoded.is_some());
        let fields = decoded.unwrap();

        prop_assert_eq!(fields.len(), 9);
        prop_assert_eq!(fields[0].value_f64, sync as f64);
        prop_assert!((fields[1].value_f64 - (temp as f64 * 0.1)).abs() < 1e-5);
        prop_assert_eq!(fields[2].value_f64, seq as f64);

        if !ax.is_nan() {
            prop_assert_eq!(fields[3].value_f64, ax as f64);
        }
        if !ay.is_nan() {
            prop_assert_eq!(fields[4].value_f64, ay as f64);
        }
        if !az.is_nan() {
            prop_assert_eq!(fields[5].value_f64, az as f64);
        }

        prop_assert_eq!(fields[6].value_f64, flag_a as f64);
        prop_assert_eq!(fields[7].value_f64, flag_b as f64);
        prop_assert_eq!(fields[8].value_f64, flag_c as f64);
    }
}
