use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Endian {
    Little,
    Big,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PrimitiveType {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    F32,
    F64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDescriptor {
    pub name: String,
    pub offset: usize,
    pub ty: PrimitiveType,
    pub scale: f64,
    pub plot_telemetry: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacketSchema {
    pub name: String,
    pub endian: Endian,
    pub total_size: usize,
    pub fields: Vec<FieldDescriptor>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DecodedField {
    pub name: String,
    pub value_f64: f64,
}

impl PacketSchema {
    pub fn dissect_frame(&self, payload: &[u8]) -> Option<Vec<DecodedField>> {
        if payload.len() < self.total_size {
            return None;
        }
        let mut out = Vec::with_capacity(self.fields.len());
        for f in &self.fields {
            let s = &payload[f.offset..];
            let raw_val = match (f.ty, self.endian) {
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
                (PrimitiveType::F32, Endian::Little) => {
                    f32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64
                }
                (PrimitiveType::F32, Endian::Big) => {
                    f32::from_be_bytes([s[0], s[1], s[2], s[3]]) as f64
                }
                (PrimitiveType::F64, Endian::Little) => {
                    f64::from_le_bytes(s[0..8].try_into().ok()?)
                }
                (PrimitiveType::F64, Endian::Big) => f64::from_be_bytes(s[0..8].try_into().ok()?),
            };
            out.push(DecodedField {
                name: f.name.clone(),
                value_f64: raw_val * f.scale,
            });
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                    scale: 1.0,
                    plot_telemetry: false,
                },
                FieldDescriptor {
                    name: "temp_c".into(),
                    offset: 2,
                    ty: PrimitiveType::I16,
                    scale: 0.01,
                    plot_telemetry: true,
                },
            ],
        };
        let frame = [0x55, 0xAA, 0xC4, 0x09]; // 0x09C4 = 2500 -> * 0.01 = 25.0 C
        let decoded = schema.dissect_frame(&frame).unwrap();
        assert_eq!(decoded[0].value_f64, 0xAA55 as f64);
        assert!((decoded[1].value_f64 - 25.0).abs() < 1e-6);
    }
}
