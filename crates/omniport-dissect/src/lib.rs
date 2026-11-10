pub mod lexer;
pub mod parser;

use serde::{Deserialize, Serialize};

pub use parser::{CStructParser, ParseError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Endian {
    Little,
    Big,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrimitiveType {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    F32,
    F64,
}

impl PrimitiveType {
    pub const fn size_bytes(&self) -> usize {
        match self {
            Self::U8 | Self::I8 => 1,
            Self::U16 | Self::I16 => 2,
            Self::U32 | Self::I32 | Self::F32 => 4,
            Self::U64 | Self::I64 | Self::F64 => 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldKind {
    Scalar,
    Array {
        count: usize,
    },
    Bitfield {
        bit_offset: u8,
        bit_width: u8,
        signed: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDescriptor {
    pub name: String,
    pub offset: usize,
    pub ty: PrimitiveType,
    pub kind: FieldKind,
    pub scale: f64,
    pub unit: Option<String>,
    pub plot_telemetry: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacketSchema {
    pub name: String,
    pub endian: Endian,
    pub total_size: usize,
    pub fields: Vec<FieldDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecodedField {
    pub name: String,
    pub value_f64: f64,
}

impl PacketSchema {
    /// Compiles a C packed struct header string into a PacketSchema at runtime
    pub fn from_c_struct(source: &str, endian: Endian) -> Result<Self, ParseError> {
        let parser = CStructParser::new(source, endian)?;
        parser.parse()
    }

    /// Returns the `[start, end)` byte span a field reads from the payload, or `None` when
    /// the descriptor is internally inconsistent (offset/size overflow, bitfield that does
    /// not fit its declared container type, zero-width bitfield).
    fn field_byte_span(f: &FieldDescriptor) -> Option<(usize, usize)> {
        let elem_size = f.ty.size_bytes();
        let span = match &f.kind {
            FieldKind::Scalar => elem_size,
            FieldKind::Array { count } => elem_size.checked_mul(*count)?,
            FieldKind::Bitfield {
                bit_offset,
                bit_width,
                ..
            } => {
                let bit_offset = *bit_offset as usize;
                let bit_width = *bit_width as usize;
                let container_bits = elem_size * 8;
                if bit_width == 0
                    || bit_width > container_bits
                    || bit_offset.checked_add(bit_width)? > container_bits
                {
                    return None;
                }
                // The whole storage unit is read to extract the bit range.
                elem_size
            }
        };
        let end = f.offset.checked_add(span)?;
        Some((f.offset, end))
    }

    /// Dissects binary frame bytes into scaled telemetry values
    pub fn dissect_frame(&self, payload: &[u8]) -> Option<Vec<DecodedField>> {
        if payload.len() < self.total_size {
            return None;
        }

        // Validate every descriptor's extent up-front: schemas can come from C source or be
        // built by callers, and a malformed one must yield `None` rather than slice/shift
        // panics while decoding an untrusted frame.
        for f in &self.fields {
            let (_, end) = Self::field_byte_span(f)?;
            if end > payload.len() {
                return None;
            }
        }

        let mut out = Vec::with_capacity(self.fields.len());

        for f in &self.fields {
            match &f.kind {
                FieldKind::Scalar => {
                    let s = &payload[f.offset..];
                    let raw_val = Self::read_primitive(s, f.ty, self.endian)?;
                    out.push(DecodedField {
                        name: f.name.clone(),
                        value_f64: raw_val * f.scale,
                    });
                }
                FieldKind::Array { count } => {
                    let elem_size = f.ty.size_bytes();
                    for i in 0..*count {
                        let off = f.offset + i * elem_size;
                        let s = &payload[off..];
                        let raw_val = Self::read_primitive(s, f.ty, self.endian)?;
                        out.push(DecodedField {
                            name: format!("{}[{}]", f.name, i),
                            value_f64: raw_val * f.scale,
                        });
                    }
                }
                FieldKind::Bitfield {
                    bit_offset,
                    bit_width,
                    signed,
                } => {
                    let s = &payload[f.offset..];
                    let raw_container = match (f.ty, self.endian) {
                        (PrimitiveType::U8 | PrimitiveType::I8, _) => s[0] as u64,
                        (PrimitiveType::U16 | PrimitiveType::I16, Endian::Little) => {
                            u16::from_le_bytes([s[0], s[1]]) as u64
                        }
                        (PrimitiveType::U16 | PrimitiveType::I16, Endian::Big) => {
                            u16::from_be_bytes([s[0], s[1]]) as u64
                        }
                        (PrimitiveType::U32 | PrimitiveType::I32, Endian::Little) => {
                            u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as u64
                        }
                        (PrimitiveType::U32 | PrimitiveType::I32, Endian::Big) => {
                            u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as u64
                        }
                        (PrimitiveType::U64 | PrimitiveType::I64, Endian::Little) => {
                            u64::from_le_bytes(s[0..8].try_into().ok()?)
                        }
                        (PrimitiveType::U64 | PrimitiveType::I64, Endian::Big) => {
                            u64::from_be_bytes(s[0..8].try_into().ok()?)
                        }
                        _ => return None,
                    };

                    let mask = if *bit_width >= 64 {
                        u64::MAX
                    } else {
                        (1u64 << bit_width) - 1
                    };
                    let raw_bits = (raw_container >> bit_offset) & mask;

                    let raw_val = if *signed {
                        if *bit_width >= 64 {
                            // A full-width signed bitfield is the two's-complement value.
                            (raw_bits as i64) as f64
                        } else {
                            let sign_bit = 1u64 << (bit_width - 1);
                            if (raw_bits & sign_bit) != 0 {
                                (raw_bits | !mask) as i64 as f64
                            } else {
                                raw_bits as i64 as f64
                            }
                        }
                    } else {
                        raw_bits as f64
                    };

                    out.push(DecodedField {
                        name: f.name.clone(),
                        value_f64: raw_val * f.scale,
                    });
                }
            }
        }

        Some(out)
    }

    fn read_primitive(s: &[u8], ty: PrimitiveType, endian: Endian) -> Option<f64> {
        let val = match (ty, endian) {
            (PrimitiveType::U8, _) => s[0] as f64,
            (PrimitiveType::I8, _) => (s[0] as i8) as f64,
            (PrimitiveType::U16, Endian::Little) => u16::from_le_bytes([s[0], s[1]]) as f64,
            (PrimitiveType::U16, Endian::Big) => u16::from_be_bytes([s[0], s[1]]) as f64,
            (PrimitiveType::I16, Endian::Little) => i16::from_le_bytes([s[0], s[1]]) as f64,
            (PrimitiveType::I16, Endian::Big) => i16::from_be_bytes([s[0], s[1]]) as f64,
            (PrimitiveType::U32, Endian::Little) => {
                u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64
            }
            (PrimitiveType::U32, Endian::Big) => {
                u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as f64
            }
            (PrimitiveType::I32, Endian::Little) => {
                i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64
            }
            (PrimitiveType::I32, Endian::Big) => {
                i32::from_be_bytes([s[0], s[1], s[2], s[3]]) as f64
            }
            (PrimitiveType::U64, Endian::Little) => {
                u64::from_le_bytes(s[0..8].try_into().ok()?) as f64
            }
            (PrimitiveType::U64, Endian::Big) => {
                u64::from_be_bytes(s[0..8].try_into().ok()?) as f64
            }
            (PrimitiveType::I64, Endian::Little) => {
                i64::from_le_bytes(s[0..8].try_into().ok()?) as f64
            }
            (PrimitiveType::I64, Endian::Big) => {
                i64::from_be_bytes(s[0..8].try_into().ok()?) as f64
            }
            (PrimitiveType::F32, Endian::Little) => {
                f32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64
            }
            (PrimitiveType::F32, Endian::Big) => {
                f32::from_be_bytes([s[0], s[1], s[2], s[3]]) as f64
            }
            (PrimitiveType::F64, Endian::Little) => f64::from_le_bytes(s[0..8].try_into().ok()?),
            (PrimitiveType::F64, Endian::Big) => f64::from_be_bytes(s[0..8].try_into().ok()?),
        };
        Some(val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dissect_frame_rejects_inconsistent_schema_without_panicking() {
        // A caller-built schema whose field extent exceeds total_size/payload length must
        // yield None instead of indexing past the payload slice.
        let schema = PacketSchema {
            name: "Inconsistent".into(),
            endian: Endian::Little,
            total_size: 1,
            fields: vec![FieldDescriptor {
                name: "wide".into(),
                offset: 0,
                ty: PrimitiveType::U32,
                kind: FieldKind::Scalar,
                scale: 1.0,
                unit: None,
                plot_telemetry: false,
            }],
        };
        assert!(schema.dissect_frame(&[0u8; 1]).is_none());
        assert!(schema.dissect_frame(&[]).is_none());
        assert!(schema.dissect_frame(&[0u8; 4]).is_some());
    }

    #[test]
    fn test_dissect_frame_rejects_invalid_bitfield_span() {
        let schema = PacketSchema {
            name: "BadBitfield".into(),
            endian: Endian::Little,
            total_size: 8,
            fields: vec![FieldDescriptor {
                name: "shift".into(),
                offset: 0,
                ty: PrimitiveType::U64,
                kind: FieldKind::Bitfield {
                    bit_offset: 64,
                    bit_width: 4,
                    signed: false,
                },
                scale: 1.0,
                unit: None,
                plot_telemetry: false,
            }],
        };
        // bit_offset + bit_width > 64 would overflow the container shift.
        assert!(schema.dissect_frame(&[0u8; 8]).is_none());
    }

    #[test]
    fn test_c_struct_dissection_and_scaling() {
        let schema = PacketSchema {
            name: "ImuPacket".into(),
            endian: Endian::Little,
            total_size: 4,
            fields: vec![
                FieldDescriptor {
                    name: "sync".into(),
                    offset: 0,
                    ty: PrimitiveType::U16,
                    kind: FieldKind::Scalar,
                    scale: 1.0,
                    unit: None,
                    plot_telemetry: false,
                },
                FieldDescriptor {
                    name: "temp_c".into(),
                    offset: 2,
                    ty: PrimitiveType::I16,
                    kind: FieldKind::Scalar,
                    scale: 0.01,
                    unit: Some("degC".into()),
                    plot_telemetry: true,
                },
            ],
        };
        let frame = [0x55, 0xAA, 0xC4, 0x09]; // 0x09C4 = 2500 -> * 0.01 = 25.0 C
        let decoded = schema.dissect_frame(&frame).unwrap();
        assert_eq!(decoded[0].value_f64, 0xAA55 as f64);
        assert!((decoded[1].value_f64 - 25.0).abs() < 1e-6);
    }

    #[test]
    fn test_parse_packed_c_struct_and_dissect() {
        let c_code = r#"
            struct __attribute__((packed)) Telemetry {
                uint16_t sync_word;
                int16_t temperature; // scale: 0.01, unit: "C", plot: true
                uint32_t timestamp;
                float accel[3];
                uint8_t status: 3;
                uint8_t error: 1;
                uint8_t mode: 4;
            };
        "#;

        let schema = PacketSchema::from_c_struct(c_code, Endian::Little).unwrap();
        assert_eq!(schema.name, "Telemetry");
        // Total size:
        // sync_word: 2B (0..2)
        // temperature: 2B (2..4)
        // timestamp: 4B (4..8)
        // accel[3]: 3 x 4B = 12B (8..20)
        // status: 3b, error: 1b, mode: 4b = 8b = 1B (20..21)
        // Total = 21B
        assert_eq!(schema.total_size, 21);

        // Build a matching payload
        let mut payload = Vec::new();
        payload.extend_from_slice(&0xAA55u16.to_le_bytes()); // sync_word
        payload.extend_from_slice(&2500i16.to_le_bytes()); // 25.00 degC
        payload.extend_from_slice(&123456u32.to_le_bytes()); // timestamp
        payload.extend_from_slice(&1.5f32.to_le_bytes()); // accel[0]
        payload.extend_from_slice(&(-2.5f32).to_le_bytes()); // accel[1]
        payload.extend_from_slice(&9.81f32.to_le_bytes()); // accel[2]

        // status: 5 (0b101), error: 1 (0b1), mode: 10 (0b1010)
        // byte = (5) | (1 << 3) | (10 << 4) = 5 | 8 | 160 = 173 (0xAD)
        payload.push(0xAD);

        let fields = schema.dissect_frame(&payload).unwrap();
        assert_eq!(fields[0].name, "sync_word");
        assert_eq!(fields[0].value_f64, 0xAA55 as f64);

        assert_eq!(fields[1].name, "temperature");
        assert!((fields[1].value_f64 - 25.0).abs() < 1e-6);

        assert_eq!(fields[2].name, "timestamp");
        assert_eq!(fields[2].value_f64, 123456.0);

        assert_eq!(fields[3].name, "accel[0]");
        assert!((fields[3].value_f64 - 1.5).abs() < 1e-6);

        assert_eq!(fields[4].name, "accel[1]");
        assert!((fields[4].value_f64 - (-2.5)).abs() < 1e-6);

        assert_eq!(fields[5].name, "accel[2]");
        assert!((fields[5].value_f64 - 9.81).abs() < 1e-4);

        assert_eq!(fields[6].name, "status");
        assert_eq!(fields[6].value_f64, 5.0);

        assert_eq!(fields[7].name, "error");
        assert_eq!(fields[7].value_f64, 1.0);

        assert_eq!(fields[8].name, "mode");
        assert_eq!(fields[8].value_f64, 10.0);
    }
}
