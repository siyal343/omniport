use omniport_core::SerialConfig;
use serde::{Deserialize, Serialize};

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
    ViewportBatch {
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
