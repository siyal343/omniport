use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioConfig {
    #[serde(default = "default_version")]
    pub version: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub endpoints: Vec<EndpointConfig>,
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
    #[serde(default)]
    pub capture: Option<CaptureConfig>,
    #[serde(default)]
    pub steps: Vec<StepConfig>,
}

fn default_version() -> String {
    "1.0".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointType {
    Mock,
    Script,
    TcpServer,
    TcpClient,
    Udp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointConfig {
    pub id: u16,
    pub name: String,
    #[serde(rename = "type")]
    pub endpoint_type: EndpointType,
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
    #[serde(default)]
    pub script_path: Option<String>,
    #[serde(default)]
    pub fuel_limit: Option<u64>,
    #[serde(default)]
    pub address: Option<String>,
}

fn default_baud_rate() -> u32 {
    115_200
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteConfig {
    #[serde(default)]
    pub id: u32,
    pub source: u16,
    pub targets: Vec<u16>,
    #[serde(default)]
    pub bidirectional: bool,
    #[serde(default)]
    pub fault: Option<FaultConfigDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FaultConfigDto {
    #[serde(default)]
    pub drop_rate: f64,
    #[serde(default)]
    pub bit_error_rate: f64,
    #[serde(default)]
    pub mean_latency_ms: f64,
    #[serde(default)]
    pub jitter_stddev_ms: f64,
    #[serde(default)]
    pub corrupt_nth_crc: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub output_pcapng: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum StepConfig {
    Sleep {
        duration_ms: u64,
    },
    Send {
        endpoint: u16,
        #[serde(default)]
        payload_hex: Option<String>,
        #[serde(default)]
        payload_text: Option<String>,
    },
    Expect {
        endpoint: u16,
        #[serde(default = "default_timeout_ms")]
        timeout_ms: u64,
        #[serde(default)]
        match_hex: Option<String>,
        #[serde(default)]
        match_text: Option<String>,
        #[serde(default)]
        min_bytes: Option<usize>,
    },
    SetPins {
        endpoint: u16,
        #[serde(default)]
        rts: Option<bool>,
        #[serde(default)]
        dtr: Option<bool>,
        #[serde(default)]
        brk: Option<bool>,
    },
    AssertPins {
        endpoint: u16,
        #[serde(default = "default_timeout_ms")]
        timeout_ms: u64,
        #[serde(default)]
        rts: Option<bool>,
        #[serde(default)]
        cts: Option<bool>,
        #[serde(default)]
        dtr: Option<bool>,
        #[serde(default)]
        dsr: Option<bool>,
        #[serde(default)]
        dcd: Option<bool>,
        #[serde(default)]
        ri: Option<bool>,
    },
    AssertMetrics {
        #[serde(default)]
        min_packets: Option<usize>,
        #[serde(default)]
        max_drops: Option<usize>,
    },
}

fn default_timeout_ms() -> u64 {
    1000
}
