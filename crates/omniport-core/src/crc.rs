use bytes::Bytes;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crc8Config {
    pub polynomial: u8,
    pub init: u8,
    pub xorout: u8,
    pub refin: bool,
    pub refout: bool,
}

impl Crc8Config {
    pub const fn new(polynomial: u8, init: u8, xorout: u8, refin: bool, refout: bool) -> Self {
        Self {
            polynomial,
            init,
            xorout,
            refin,
            refout,
        }
    }

    pub fn compute(&self, data: &[u8]) -> u8 {
        let mut reg = self.init;
        for &byte in data {
            let b = if self.refin {
                byte.reverse_bits()
            } else {
                byte
            };
            reg ^= b;
            for _ in 0..8 {
                if (reg & 0x80) != 0 {
                    reg = (reg << 1) ^ self.polynomial;
                } else {
                    reg <<= 1;
                }
            }
        }
        if self.refout {
            reg = reg.reverse_bits();
        }
        reg ^ self.xorout
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crc16Config {
    pub polynomial: u16,
    pub init: u16,
    pub xorout: u16,
    pub refin: bool,
    pub refout: bool,
    pub little_endian: bool,
}

impl Crc16Config {
    pub const fn new(
        polynomial: u16,
        init: u16,
        xorout: u16,
        refin: bool,
        refout: bool,
        little_endian: bool,
    ) -> Self {
        Self {
            polynomial,
            init,
            xorout,
            refin,
            refout,
            little_endian,
        }
    }

    pub fn compute(&self, data: &[u8]) -> u16 {
        let mut reg = self.init;
        for &byte in data {
            let b = if self.refin {
                byte.reverse_bits()
            } else {
                byte
            };
            reg ^= (b as u16) << 8;
            for _ in 0..8 {
                if (reg & 0x8000) != 0 {
                    reg = (reg << 1) ^ self.polynomial;
                } else {
                    reg <<= 1;
                }
            }
        }
        if self.refout {
            reg = reg.reverse_bits();
        }
        reg ^ self.xorout
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crc32Config {
    pub polynomial: u32,
    pub init: u32,
    pub xorout: u32,
    pub refin: bool,
    pub refout: bool,
    pub little_endian: bool,
}

impl Crc32Config {
    pub const fn new(
        polynomial: u32,
        init: u32,
        xorout: u32,
        refin: bool,
        refout: bool,
        little_endian: bool,
    ) -> Self {
        Self {
            polynomial,
            init,
            xorout,
            refin,
            refout,
            little_endian,
        }
    }

    pub fn compute(&self, data: &[u8]) -> u32 {
        let mut reg = self.init;
        for &byte in data {
            let b = if self.refin {
                byte.reverse_bits()
            } else {
                byte
            };
            reg ^= (b as u32) << 24;
            for _ in 0..8 {
                if (reg & 0x8000_0000) != 0 {
                    reg = (reg << 1) ^ self.polynomial;
                } else {
                    reg <<= 1;
                }
            }
        }
        if self.refout {
            reg = reg.reverse_bits();
        }
        reg ^ self.xorout
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrcAlgorithm {
    Crc8(Crc8Config),
    Crc16(Crc16Config),
    Crc32(Crc32Config),
}

impl CrcAlgorithm {
    /// Standard CRC-8 (SMBus / ATM: poly 0x07, init 0x00, xorout 0x00)
    pub const fn crc8() -> Self {
        Self::Crc8(Crc8Config::new(0x07, 0x00, 0x00, false, false))
    }

    /// CRC-8 SAE J1850 (poly 0x1D, init 0xFF, xorout 0xFF)
    pub const fn crc8_sae_j1850() -> Self {
        Self::Crc8(Crc8Config::new(0x1D, 0xFF, 0xFF, false, false))
    }

    /// CRC-8 Dallas/Maxim (poly 0x31, init 0x00, xorout 0x00, reflected)
    pub const fn crc8_maxim() -> Self {
        Self::Crc8(Crc8Config::new(0x31, 0x00, 0x00, true, true))
    }

    /// CRC-16 Modbus (poly 0x8005, init 0xFFFF, xorout 0x0000, reflected, little-endian)
    pub const fn crc16_modbus() -> Self {
        Self::Crc16(Crc16Config::new(0x8005, 0xFFFF, 0x0000, true, true, true))
    }

    /// CRC-16 CCITT-False (poly 0x1021, init 0xFFFF, xorout 0x0000, big-endian)
    pub const fn crc16_ccitt() -> Self {
        Self::Crc16(Crc16Config::new(
            0x1021, 0xFFFF, 0x0000, false, false, false,
        ))
    }

    /// CRC-16 XMODEM (poly 0x1021, init 0x0000, xorout 0x0000, big-endian)
    pub const fn crc16_xmodem() -> Self {
        Self::Crc16(Crc16Config::new(
            0x1021, 0x0000, 0x0000, false, false, false,
        ))
    }

    /// CRC-32 (IEEE 802.3 / ISO 3309 / Ethernet: poly 0x04C11DB7, init 0xFFFFFFFF, xorout 0xFFFFFFFF, reflected, little-endian)
    pub const fn crc32() -> Self {
        Self::Crc32(Crc32Config::new(
            0x04C1_1DB7,
            0xFFFF_FFFF,
            0xFFFF_FFFF,
            true,
            true,
            true,
        ))
    }

    /// CRC-32 MPEG-2 (poly 0x04C11DB7, init 0xFFFFFFFF, xorout 0x00000000, big-endian)
    pub const fn crc32_mpeg2() -> Self {
        Self::Crc32(Crc32Config::new(
            0x04C1_1DB7,
            0xFFFF_FFFF,
            0x0000_0000,
            false,
            false,
            false,
        ))
    }

    pub fn checksum_len(&self) -> usize {
        match self {
            Self::Crc8(_) => 1,
            Self::Crc16(_) => 2,
            Self::Crc32(_) => 4,
        }
    }

    pub fn compute(&self, data: &[u8]) -> u32 {
        match self {
            Self::Crc8(cfg) => cfg.compute(data) as u32,
            Self::Crc16(cfg) => cfg.compute(data) as u32,
            Self::Crc32(cfg) => cfg.compute(data),
        }
    }

    /// Appends the computed checksum to the data slice in the algorithm's configured endianness.
    pub fn append_checksum(&self, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len() + self.checksum_len());
        out.extend_from_slice(data);
        match self {
            Self::Crc8(cfg) => {
                out.push(cfg.compute(data));
            }
            Self::Crc16(cfg) => {
                let crc = cfg.compute(data);
                if cfg.little_endian {
                    out.extend_from_slice(&crc.to_le_bytes());
                } else {
                    out.extend_from_slice(&crc.to_be_bytes());
                }
            }
            Self::Crc32(cfg) => {
                let crc = cfg.compute(data);
                if cfg.little_endian {
                    out.extend_from_slice(&crc.to_le_bytes());
                } else {
                    out.extend_from_slice(&crc.to_be_bytes());
                }
            }
        }
        out
    }

    /// Validates whether the packet's trailing bytes match the expected CRC checksum of its preceding payload.
    pub fn validate(&self, packet_with_crc: &[u8]) -> bool {
        let len = self.checksum_len();
        if packet_with_crc.len() < len {
            return false;
        }
        let payload = &packet_with_crc[..packet_with_crc.len() - len];
        let expected_checksum = &packet_with_crc[packet_with_crc.len() - len..];

        match self {
            Self::Crc8(cfg) => {
                let computed = cfg.compute(payload);
                expected_checksum[0] == computed
            }
            Self::Crc16(cfg) => {
                let computed = cfg.compute(payload);
                let actual = if cfg.little_endian {
                    u16::from_le_bytes([expected_checksum[0], expected_checksum[1]])
                } else {
                    u16::from_be_bytes([expected_checksum[0], expected_checksum[1]])
                };
                computed == actual
            }
            Self::Crc32(cfg) => {
                let computed = cfg.compute(payload);
                let actual = if cfg.little_endian {
                    u32::from_le_bytes([
                        expected_checksum[0],
                        expected_checksum[1],
                        expected_checksum[2],
                        expected_checksum[3],
                    ])
                } else {
                    u32::from_be_bytes([
                        expected_checksum[0],
                        expected_checksum[1],
                        expected_checksum[2],
                        expected_checksum[3],
                    ])
                };
                computed == actual
            }
        }
    }

    /// Verifies the checksum and returns the stripped payload if valid.
    pub fn strip_crc(&self, packet_with_crc: &[u8]) -> Option<Bytes> {
        if self.validate(packet_with_crc) {
            let payload_len = packet_with_crc.len() - self.checksum_len();
            Some(Bytes::copy_from_slice(&packet_with_crc[..payload_len]))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_VECTOR: &[u8] = b"123456789";

    #[test]
    fn test_crc8_standard_test_vector() {
        let crc = CrcAlgorithm::crc8();
        assert_eq!(crc.compute(TEST_VECTOR), 0xF4);
    }

    #[test]
    fn test_crc8_sae_j1850_test_vector() {
        let crc = CrcAlgorithm::crc8_sae_j1850();
        assert_eq!(crc.compute(TEST_VECTOR), 0x4B);
    }

    #[test]
    fn test_crc8_maxim_test_vector() {
        let crc = CrcAlgorithm::crc8_maxim();
        assert_eq!(crc.compute(TEST_VECTOR), 0xA1);
    }

    #[test]
    fn test_crc16_modbus_test_vector() {
        let crc = CrcAlgorithm::crc16_modbus();
        assert_eq!(crc.compute(TEST_VECTOR), 0x4B37);
    }

    #[test]
    fn test_crc16_ccitt_test_vector() {
        let crc = CrcAlgorithm::crc16_ccitt();
        assert_eq!(crc.compute(TEST_VECTOR), 0x29B1);
    }

    #[test]
    fn test_crc16_xmodem_test_vector() {
        let crc = CrcAlgorithm::crc16_xmodem();
        assert_eq!(crc.compute(TEST_VECTOR), 0x31C3);
    }

    #[test]
    fn test_crc32_standard_test_vector() {
        let crc = CrcAlgorithm::crc32();
        assert_eq!(crc.compute(TEST_VECTOR), 0xCBF4_3926);
    }

    #[test]
    fn test_crc32_mpeg2_test_vector() {
        let crc = CrcAlgorithm::crc32_mpeg2();
        assert_eq!(crc.compute(TEST_VECTOR), 0x0376_E6E7);
    }

    #[test]
    fn test_crc_append_and_validation() {
        let algos = [
            CrcAlgorithm::crc8(),
            CrcAlgorithm::crc8_sae_j1850(),
            CrcAlgorithm::crc8_maxim(),
            CrcAlgorithm::crc16_modbus(),
            CrcAlgorithm::crc16_ccitt(),
            CrcAlgorithm::crc16_xmodem(),
            CrcAlgorithm::crc32(),
            CrcAlgorithm::crc32_mpeg2(),
        ];

        let payload = b"OmniPort-Telemetry-Payload-123456";

        for algo in algos {
            let frame = algo.append_checksum(payload);
            assert!(
                algo.validate(&frame),
                "Algorithm {:?} failed validation",
                algo
            );

            let stripped = algo.strip_crc(&frame).expect("Strip should succeed");
            assert_eq!(&stripped[..], payload);

            // Corrupt single bit
            let mut corrupted = frame.clone();
            let last_idx = corrupted.len() - 1;
            corrupted[last_idx] ^= 0x01;
            assert!(
                !algo.validate(&corrupted),
                "Corrupted packet should fail validation"
            );
        }
    }
}
