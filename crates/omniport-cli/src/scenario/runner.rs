use super::error::ScenarioError;
use super::schema::{EndpointType, FaultConfigDto, ScenarioConfig, StepConfig};
use crate::hex_utils::{decode_hex, hex_matches};
use bytes::Bytes;
use omniport_capture::pcapng::PcapngWriter;
use omniport_core::fault::FaultConfig;
use omniport_core::routing::{BackpressurePolicy, RoutingGraph};
use omniport_core::topology::{Request as TopologyRequest, Topology};
use omniport_core::{AsyncEndpoint, EndpointId, FrameFlags, PinState, SerialConfig, SerialEvent};
use omniport_script::engine::ScriptConfig;
use omniport_script::{ScriptEndpoint, ScriptEngine};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepReport {
    pub index: usize,
    pub action: String,
    pub description: String,
    pub duration_ms: f64,
    pub passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioReport {
    pub success: bool,
    pub scenario_name: String,
    pub total_steps: usize,
    pub passed_steps: usize,
    pub failed_steps: usize,
    pub duration_ms: f64,
    pub steps: Vec<StepReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pcapng_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct ScenarioRunner {
    config: ScenarioConfig,
    topology: Topology,
    router: Arc<RoutingGraph>,
    pin_states: Arc<Mutex<HashMap<EndpointId, PinState>>>,
    pcapng_writer: Option<Arc<Mutex<PcapngWriter<File>>>>,
    bridge_tasks: Vec<JoinHandle<()>>,
    cancel_txs: Vec<tokio::sync::oneshot::Sender<()>>,
    packets_sent: Arc<AtomicUsize>,
    packets_received: Arc<AtomicUsize>,
}

impl ScenarioRunner {
    /// Loads scenario configuration from a YAML file
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ScenarioError> {
        let content = std::fs::read_to_string(path.as_ref()).map_err(|e| {
            ScenarioError::ConfigError(format!("Failed to read scenario file: {e}"))
        })?;
        Self::from_yaml(&content)
    }

    /// Parses scenario configuration from a YAML string
    pub fn from_yaml(yaml_str: &str) -> Result<Self, ScenarioError> {
        let config: ScenarioConfig =
            serde_yaml::from_str(yaml_str).map_err(|e| ScenarioError::ParseError(e.to_string()))?;
        Self::new(config)
    }

    /// Creates and initializes a new scenario runner with all endpoints and routes
    pub fn new(config: ScenarioConfig) -> Result<Self, ScenarioError> {
        let mut topology = Topology::new();
        let pin_states = Arc::new(Mutex::new(HashMap::new()));
        let mut pending_scripts: Vec<(EndpointId, ScriptEndpoint)> = Vec::new();

        // 1. Register endpoints through the canonical Topology seam
        for ep in &config.endpoints {
            let serial_cfg = SerialConfig {
                baud_rate: ep.baud_rate,
                ..Default::default()
            };
            topology.apply(TopologyRequest::RegisterEndpoint {
                id: Some(ep.id),
                name: ep.name.clone(),
                port_type: endpoint_type_name(ep.endpoint_type).to_string(),
                config: serial_cfg,
                queue_capacity: 1024,
                backpressure: BackpressurePolicy::DropNewest,
            })?;

            pin_states
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(ep.id, PinState::empty());

            // Validate and build endpoint-specific workers before spawning any tasks
            match ep.endpoint_type {
                EndpointType::Mock => {
                    // Mock endpoints are driven directly by the runner
                }
                EndpointType::Script => {
                    let script_path = ep.script_path.as_deref().ok_or_else(|| {
                        ScenarioError::ConfigError(format!(
                            "Endpoint '{}' ({}) requires 'script_path'",
                            ep.name, ep.id
                        ))
                    })?;
                    let fuel = ep.fuel_limit.unwrap_or(1_000_000);

                    let mut engine = ScriptEngine::new(ScriptConfig {
                        max_fuel: fuel,
                        ..Default::default()
                    })
                    .map_err(|e| ScenarioError::ScriptError(e.to_string()))?;

                    let script_code = std::fs::read_to_string(script_path).map_err(|e| {
                        ScenarioError::ScriptError(format!(
                            "Failed to read script file {script_path}: {e}"
                        ))
                    })?;
                    engine
                        .load_script(&script_code)
                        .map_err(|e| ScenarioError::ScriptError(e.to_string()))?;

                    let script_ep = ScriptEndpoint::new(ep.id, &ep.name, serial_cfg, engine);
                    pending_scripts.push((ep.id, script_ep));
                }
                EndpointType::TcpServer | EndpointType::TcpClient | EndpointType::Udp => {
                    // Network endpoints can be wired when live network testing is configured
                }
            }
        }

        // 2. Configure routes and fault profiles through Topology
        for route in &config.routes {
            let fault = route.fault.as_ref().map(convert_fault_config).transpose()?;
            topology.apply(TopologyRequest::AddRoute {
                source_id: route.source,
                target_ids: route.targets.clone(),
                bidirectional: route.bidirectional,
                fault,
            })?;
        }

        // 3. Initialize PCAPNG capture if enabled
        let pcapng_writer = if let Some(ref capture) = config.capture {
            if capture.enabled {
                if let Some(parent) = Path::new(&capture.output_pcapng).parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let file = File::create(&capture.output_pcapng)?;
                let mut writer = PcapngWriter::new(file);
                writer
                    .write_header("OmniPort CI Scenario Capture")
                    .map_err(|e| ScenarioError::ConfigError(e.to_string()))?;
                Some(Arc::new(Mutex::new(writer)))
            } else {
                None
            }
        } else {
            None
        };

        // 4. Spawn endpoint bridges only after all config/topology validation succeeds
        let router = topology.graph();
        let mut bridge_tasks = Vec::with_capacity(pending_scripts.len());
        let mut cancel_txs = Vec::with_capacity(pending_scripts.len());
        for (ep_id, script_ep) in pending_scripts {
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
            cancel_txs.push(cancel_tx);
            bridge_tasks.push(spawn_endpoint_bridge(
                script_ep,
                ep_id,
                Arc::clone(&router),
                cancel_rx,
            ));
        }

        Ok(Self {
            config,
            topology,
            router,
            pin_states,
            pcapng_writer,
            bridge_tasks,
            cancel_txs,
            packets_sent: Arc::new(AtomicUsize::new(0)),
            packets_received: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// Runs all steps in the scenario and returns a comprehensive scenario report
    pub async fn run(&mut self) -> ScenarioReport {
        let start_time = Instant::now();
        let total_steps = self.config.steps.len();
        let mut step_reports = Vec::with_capacity(total_steps);
        let mut failed = false;
        let mut overall_err = None;

        let steps = self.config.steps.clone();
        for (idx, step) in steps.iter().enumerate() {
            let step_idx = idx + 1;
            let step_start = Instant::now();
            let (action_name, desc, step_res) = self.execute_step(step).await;
            let step_dur = step_start.elapsed().as_secs_f64() * 1000.0;

            match step_res {
                Ok(()) => {
                    step_reports.push(StepReport {
                        index: step_idx,
                        action: action_name,
                        description: desc,
                        duration_ms: step_dur,
                        passed: true,
                        error: None,
                    });
                }
                Err(err) => {
                    let err_msg = err.to_string();
                    step_reports.push(StepReport {
                        index: step_idx,
                        action: action_name,
                        description: desc,
                        duration_ms: step_dur,
                        passed: false,
                        error: Some(err_msg.clone()),
                    });
                    failed = true;
                    overall_err = Some(err_msg);
                    break;
                }
            }
        }

        // Clean tear down of background bridge workers
        for tx in self.cancel_txs.drain(..) {
            let _ = tx.send(());
        }
        for task in self.bridge_tasks.drain(..) {
            task.abort();
        }

        let passed_steps = step_reports.iter().filter(|s| s.passed).count();
        let failed_steps = step_reports.iter().filter(|s| !s.passed).count();
        let duration_ms = start_time.elapsed().as_secs_f64() * 1000.0;
        let pcapng_output = self
            .config
            .capture
            .as_ref()
            .filter(|c| c.enabled)
            .map(|c| c.output_pcapng.clone());

        ScenarioReport {
            success: !failed,
            scenario_name: self.config.name.clone(),
            total_steps,
            passed_steps,
            failed_steps,
            duration_ms,
            steps: step_reports,
            pcapng_output,
            error: overall_err,
        }
    }

    #[allow(clippy::collapsible_if)]
    async fn execute_step(
        &mut self,
        step: &StepConfig,
    ) -> (String, String, Result<(), ScenarioError>) {
        match step {
            StepConfig::Sleep { duration_ms } => {
                let desc = format!("Sleep {duration_ms}ms");
                tokio::time::sleep(Duration::from_millis(*duration_ms)).await;
                ("sleep".into(), desc, Ok(()))
            }

            StepConfig::Send {
                endpoint,
                payload_hex,
                payload_text,
            } => {
                if let Err(e) = self.ensure_endpoint(*endpoint) {
                    return ("send".into(), format!("Send on ep {endpoint}"), Err(e));
                }
                let payload = if let Some(hex_str) = payload_hex {
                    match decode_hex(hex_str) {
                        Ok(b) => Bytes::from(b),
                        Err(e) => {
                            return (
                                "send".into(),
                                format!("Send hex on ep {endpoint}"),
                                Err(ScenarioError::ConfigError(e)),
                            );
                        }
                    }
                } else if let Some(text) = payload_text {
                    Bytes::copy_from_slice(text.as_bytes())
                } else {
                    return (
                        "send".into(),
                        format!("Send on ep {endpoint}"),
                        Err(ScenarioError::ConfigError(
                            "No payload provided in send step".into(),
                        )),
                    );
                };

                let pins = self.get_pins(*endpoint);
                let now_ns = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;

                let event = SerialEvent {
                    timestamp_ns: now_ns,
                    source_id: *endpoint,
                    flags: FrameFlags::empty(),
                    pins,
                    payload: payload.clone(),
                };

                if let Some(ref writer_mutex) = self.pcapng_writer {
                    if let Ok(mut writer) = writer_mutex.lock() {
                        let _ = writer.write_packet(now_ns, &payload, 0);
                    }
                }

                let desc = format!("Send {} bytes on ep {}", payload.len(), endpoint);
                match self.router.route(event) {
                    Ok(_) => {
                        self.packets_sent.fetch_add(1, Ordering::Relaxed);
                        ("send".into(), desc, Ok(()))
                    }
                    Err(e) => ("send".into(), desc, Err(ScenarioError::RoutingError(e))),
                }
            }

            StepConfig::Expect {
                endpoint,
                timeout_ms,
                match_hex,
                match_text,
                min_bytes,
            } => {
                let desc = format!("Expect on ep {endpoint} (timeout: {timeout_ms}ms)");
                if let Err(e) = self.ensure_endpoint(*endpoint) {
                    return ("expect".into(), desc, Err(e));
                }
                let timeout = Duration::from_millis(*timeout_ms);
                let ep_id = *endpoint;

                let recv_fut = async {
                    loop {
                        let event = self.router.recv(ep_id).await?;
                        self.packets_received.fetch_add(1, Ordering::Relaxed);

                        if let Some(ref writer_mutex) = self.pcapng_writer {
                            if let Ok(mut writer) = writer_mutex.lock() {
                                let _ = writer.write_packet(event.timestamp_ns, &event.payload, 0);
                            }
                        }

                        // Validate minimum byte count
                        if let Some(min) = min_bytes {
                            if event.payload.len() < *min {
                                continue;
                            }
                        }

                        // Validate hex match
                        if let Some(hex_pat) = match_hex {
                            match hex_matches(&event.payload, hex_pat) {
                                Ok(true) => {}
                                Ok(false) => continue,
                                Err(e) => return Err(ScenarioError::ConfigError(e)),
                            }
                        }

                        // Validate text match
                        if let Some(text_pat) = match_text {
                            let text = String::from_utf8_lossy(&event.payload);
                            if !text.contains(text_pat) {
                                continue;
                            }
                        }

                        // Matched all criteria
                        return Ok(());
                    }
                };

                match tokio::time::timeout(timeout, recv_fut).await {
                    Ok(Ok(())) => ("expect".into(), desc, Ok(())),
                    Ok(Err(e)) => ("expect".into(), desc, Err(e)),
                    Err(_) => (
                        "expect".into(),
                        desc,
                        Err(ScenarioError::Timeout(format!(
                            "Timed out after {timeout_ms}ms waiting for matching event on endpoint {endpoint}"
                        ))),
                    ),
                }
            }

            StepConfig::SetPins {
                endpoint,
                rts,
                dtr,
                brk,
            } => {
                let desc = format!("Set pins on ep {endpoint} (RTS={:?}, DTR={:?})", rts, dtr);
                if let Err(e) = self.ensure_endpoint(*endpoint) {
                    return ("set_pins".into(), desc, Err(e));
                }
                let mut current = self.get_pins(*endpoint);
                if let Some(v) = rts {
                    current.set(PinState::RTS, *v);
                }
                if let Some(v) = dtr {
                    current.set(PinState::DTR, *v);
                }
                if let Some(v) = brk {
                    current.set(PinState::BREAK, *v);
                }
                self.set_pins(*endpoint, current);

                let now_ns = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;

                let event = SerialEvent {
                    timestamp_ns: now_ns,
                    source_id: *endpoint,
                    flags: FrameFlags::PIN_CHANGE,
                    pins: current,
                    payload: Bytes::new(),
                };

                let _ = self.router.route(event);
                ("set_pins".into(), desc, Ok(()))
            }

            StepConfig::AssertPins {
                endpoint,
                timeout_ms: _,
                rts,
                cts,
                dtr,
                dsr,
                dcd,
                ri,
            } => {
                let desc = format!("Assert pins on ep {endpoint}");
                if let Err(e) = self.ensure_endpoint(*endpoint) {
                    return ("assert_pins".into(), desc, Err(e));
                }
                let current = self.get_pins(*endpoint);
                let mut mismatches = Vec::new();

                if let Some(expected) = rts {
                    if current.contains(PinState::RTS) != *expected {
                        mismatches.push(format!(
                            "RTS (expected {expected}, got {})",
                            current.contains(PinState::RTS)
                        ));
                    }
                }
                if let Some(expected) = cts {
                    if current.contains(PinState::CTS) != *expected {
                        mismatches.push(format!(
                            "CTS (expected {expected}, got {})",
                            current.contains(PinState::CTS)
                        ));
                    }
                }
                if let Some(expected) = dtr {
                    if current.contains(PinState::DTR) != *expected {
                        mismatches.push(format!(
                            "DTR (expected {expected}, got {})",
                            current.contains(PinState::DTR)
                        ));
                    }
                }
                if let Some(expected) = dsr {
                    if current.contains(PinState::DSR) != *expected {
                        mismatches.push(format!(
                            "DSR (expected {expected}, got {})",
                            current.contains(PinState::DSR)
                        ));
                    }
                }
                if let Some(expected) = dcd {
                    if current.contains(PinState::DCD) != *expected {
                        mismatches.push(format!(
                            "DCD (expected {expected}, got {})",
                            current.contains(PinState::DCD)
                        ));
                    }
                }
                if let Some(expected) = ri {
                    if current.contains(PinState::RI) != *expected {
                        mismatches.push(format!(
                            "RI (expected {expected}, got {})",
                            current.contains(PinState::RI)
                        ));
                    }
                }

                let desc = format!("Assert pins on ep {endpoint}");
                if mismatches.is_empty() {
                    ("assert_pins".into(), desc, Ok(()))
                } else {
                    (
                        "assert_pins".into(),
                        desc,
                        Err(ScenarioError::AssertionFailed(format!(
                            "Pin assertion failed on endpoint {endpoint}: {}",
                            mismatches.join(", ")
                        ))),
                    )
                }
            }

            StepConfig::AssertMetrics {
                min_packets,
                max_drops,
            } => {
                let desc = "Assert scenario metrics".to_string();
                let sent = self.packets_sent.load(Ordering::Relaxed);
                let recvd = self.packets_received.load(Ordering::Relaxed);

                if let Some(min) = min_packets {
                    if recvd < *min {
                        return (
                            "assert_metrics".into(),
                            desc,
                            Err(ScenarioError::AssertionFailed(format!(
                                "Received packet count {recvd} below minimum {min}"
                            ))),
                        );
                    }
                }

                if let Some(max_d) = max_drops {
                    let drops = sent.saturating_sub(recvd);
                    if drops > *max_d {
                        return (
                            "assert_metrics".into(),
                            desc,
                            Err(ScenarioError::AssertionFailed(format!(
                                "Dropped packet count {drops} exceeds maximum {max_d}"
                            ))),
                        );
                    }
                }

                ("assert_metrics".into(), desc, Ok(()))
            }
        }
    }

    /// Rejects steps that reference an endpoint which was never declared.
    /// Previously `send` on a typo'd/undeclared endpoint silently succeeded
    /// (the routing graph returned zero destinations), making a broken
    /// scenario report PASS.
    fn ensure_endpoint(&self, ep_id: EndpointId) -> Result<(), ScenarioError> {
        if self.topology.has_endpoint(ep_id) {
            Ok(())
        } else {
            Err(ScenarioError::ConfigError(format!(
                "step references undeclared endpoint {ep_id}"
            )))
        }
    }

    fn get_pins(&self, ep_id: EndpointId) -> PinState {
        let guard = self.pin_states.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(&ep_id).copied().unwrap_or_default()
    }

    fn set_pins(&self, ep_id: EndpointId, pins: PinState) {
        let mut guard = self.pin_states.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(ep_id, pins);
    }
}

fn endpoint_type_name(kind: EndpointType) -> &'static str {
    match kind {
        EndpointType::Mock => "mock",
        EndpointType::Script => "script",
        EndpointType::TcpServer => "tcp_server",
        EndpointType::TcpClient => "tcp_client",
        EndpointType::Udp => "udp",
    }
}

/// Maps the declarative fault profile onto the core `FaultConfig`.
///
/// Every documented YAML field must either take effect or be rejected with a
/// clear error — silently ignoring a fault (as `drop_rate` used to be) makes a
/// failing CI scenario look green.
fn convert_fault_config(dto: &FaultConfigDto) -> Result<FaultConfig, ScenarioError> {
    fn check_unit_interval(field: &str, value: f64) -> Result<(), ScenarioError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(ScenarioError::ConfigError(format!(
                "fault.{field} must be a probability in 0.0..=1.0, got {value}"
            )));
        }
        Ok(())
    }
    fn check_non_negative(field: &str, value: f64) -> Result<(), ScenarioError> {
        if !value.is_finite() || value < 0.0 {
            return Err(ScenarioError::ConfigError(format!(
                "fault.{field} must be a finite non-negative number, got {value}"
            )));
        }
        Ok(())
    }

    check_unit_interval("drop_rate", dto.drop_rate)?;
    check_unit_interval("bit_error_rate", dto.bit_error_rate)?;
    check_non_negative("mean_latency_ms", dto.mean_latency_ms)?;
    check_non_negative("jitter_stddev_ms", dto.jitter_stddev_ms)?;

    let mut cfg = FaultConfig::default();

    // `FaultConfig` models an edge either as connected or fully disconnected;
    // there is no per-frame Bernoulli drop. A fractional drop_rate would be
    // silently ignored, so fail loudly instead of pretending it was applied.
    if dto.drop_rate >= 1.0 {
        cfg.disconnected = true;
    } else if dto.drop_rate > 0.0 {
        return Err(ScenarioError::ConfigError(format!(
            "fault.drop_rate {} is not supported: only 0.0 (no drops) and 1.0 \
             (full disconnect) are currently modelled by the fault injector",
            dto.drop_rate
        )));
    }

    // `mean_latency_ms` is the mean added propagation delay and maps onto the
    // Gaussian jitter mean; `jitter_stddev_ms` is its standard deviation.
    cfg.bit_error_rate = dto.bit_error_rate;
    if dto.mean_latency_ms > 0.0 {
        cfg.jitter_mean = Duration::from_secs_f64(dto.mean_latency_ms / 1000.0);
    }
    if dto.jitter_stddev_ms > 0.0 {
        cfg.jitter_std_dev = Duration::from_secs_f64(dto.jitter_stddev_ms / 1000.0);
    }
    if dto.corrupt_nth_crc > 0 {
        cfg.corrupt_nth_crc = Some(dto.corrupt_nth_crc as u64);
    }
    Ok(cfg)
}

fn spawn_endpoint_bridge<E: AsyncEndpoint>(
    mut endpoint: E,
    ep_id: EndpointId,
    router: Arc<RoutingGraph>,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut cancel_rx => break,
                router_res = router.recv(ep_id) => {
                    match router_res {
                        Ok(evt) => {
                            if endpoint.send(evt).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                ep_res = endpoint.recv() => {
                    match ep_res {
                        Ok(evt) => {
                            let _ = router.route(evt);
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    })
}
