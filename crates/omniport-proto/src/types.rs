use omniport_core::SerialConfig;
use serde::{Deserialize, Serialize};

/// Maximum raw size of one ICD §2 viewport batch sent over the control plane.
///
/// The control plane is JSON framed by [`crate::codec::MAX_FRAME_SIZE`] (16 MiB),
/// so this cap keeps the base64-encoded batch (~4/3 expansion) inside the frame
/// budget with room for the JSON envelope. Larger requests are refused explicitly
/// instead of failing later inside the codec with a less actionable error.
pub const MAX_VIEWPORT_BATCH_BYTES: usize = 8 * 1024 * 1024;

/// Serializable serial configuration DTO
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialConfigDto {
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: u8,
}

impl From<SerialConfig> for SerialConfigDto {
    fn from(c: SerialConfig) -> Self {
        Self {
            baud_rate: c.baud_rate,
            data_bits: c.data_bits,
            stop_bits: c.stop_bits,
            parity: c.parity,
        }
    }
}

impl From<SerialConfigDto> for SerialConfig {
    fn from(dto: SerialConfigDto) -> Self {
        Self {
            baud_rate: dto.baud_rate,
            data_bits: dto.data_bits,
            stop_bits: dto.stop_bits,
            parity: dto.parity,
        }
    }
}

impl Default for SerialConfigDto {
    fn default() -> Self {
        Self {
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
        }
    }
}

/// Information describing an active serial or virtual endpoint
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointInfo {
    pub id: u16,
    pub name: String,
    pub port_type: String,
    pub baud_rate: u32,
    pub is_active: bool,
}

/// Control-plane request message from client to daemon
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    HealthCheck,
    ListEndpoints,
    CreateVirtualPair {
        port_a: String,
        port_b: String,
        config: SerialConfigDto,
    },
    RemoveEndpoint {
        id: u16,
    },
    AddRoute {
        source_id: u16,
        target_ids: Vec<u16>,
        bidirectional: bool,
    },
    RemoveRoute {
        route_id: u32,
    },
    ReadViewport {
        start_row: u64,
        row_count: u32,
    },
    CaptureStatus,
    Shutdown,
}

/// Control-plane response message from daemon to client
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    Health {
        status: String,
        uptime_secs: u64,
        active_endpoints: u32,
        active_routes: u32,
    },
    Endpoints(Vec<EndpointInfo>),
    VirtualPairCreated {
        pair_id: u32,
        port_a: String,
        port_b: String,
        success: bool,
        message: String,
    },
    RouteAdded {
        route_id: u32,
        success: bool,
        message: String,
    },
    /// ICD §2 encoded binary viewport batch.
    ///
    /// The control-plane codec is JSON: a JSON array of byte values would inflate
    /// a payload several-fold, so `bytes` travels as a base64 string on the wire
    /// (see [`Response::viewport_batch`] for the size policy).
    ViewportBatch {
        #[serde(with = "viewport_bytes")]
        bytes: Vec<u8>,
    },
    CaptureStatus {
        total_packets: u64,
        dropped_frames: u64,
    },
    Success {
        message: String,
    },
    Error {
        code: u32,
        message: String,
    },
}

impl Response {
    /// Wraps an ICD §2 viewport batch, enforcing the control-plane size budget.
    ///
    /// Base64 expansion is at most 4/3, so [`MAX_VIEWPORT_BATCH_BYTES`] (8 MiB)
    /// becomes ~10.7 MiB on the wire and still fits the 16 MiB JSON frame limit.
    /// Oversized batches are reported as a 413 protocol error rather than a
    /// generic codec failure at a later layer.
    pub fn viewport_batch(bytes: Vec<u8>) -> Self {
        if bytes.len() > MAX_VIEWPORT_BATCH_BYTES {
            return Self::Error {
                code: 413,
                message: format!(
                    "viewport batch of {} bytes exceeds the {} byte control-plane limit",
                    bytes.len(),
                    MAX_VIEWPORT_BATCH_BYTES
                ),
            };
        }
        Self::ViewportBatch { bytes }
    }
}

/// Base64 serde for raw viewport bytes inside the JSON control-plane payload.
mod viewport_bytes {
    use base64::Engine as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}
