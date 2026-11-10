use crate::error::{HalError, Result};
use bytes::Bytes;
use omniport_core::TokenBucketThrottler;
use omniport_core::{AsyncEndpoint, EndpointId, FrameFlags, PinState, SerialConfig, SerialEvent};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, broadcast};

// Linux Modem Control Line Constants (from <linux/termios.h>)
pub const TIOCM_LE: u32 = 0x001; // Line Enable
pub const TIOCM_DTR: u32 = 0x002; // Data Terminal Ready
pub const TIOCM_RTS: u32 = 0x004; // Request To Send
pub const TIOCM_ST: u32 = 0x008; // Secondary TX
pub const TIOCM_SR: u32 = 0x010; // Secondary RX
pub const TIOCM_CTS: u32 = 0x020; // Clear To Send
pub const TIOCM_CAR: u32 = 0x040; // Carrier Detect (DCD)
pub const TIOCM_RNG: u32 = 0x080; // Ring Indicator (RI)
pub const TIOCM_DSR: u32 = 0x100; // Data Set Ready
pub const TIOCM_CD: u32 = TIOCM_CAR;
pub const TIOCM_RI: u32 = TIOCM_RNG;

// Linux Serial Ioctl Command Constants
pub const TCGETS: u32 = 0x5401;
pub const TCSETS: u32 = 0x5402;
pub const TCSBRK: u32 = 0x5409;
pub const TIOCMGET: u32 = 0x5415;
pub const TIOCMBIS: u32 = 0x5416;
pub const TIOCMBIC: u32 = 0x5417;
pub const TIOCMSET: u32 = 0x5418;
pub const TIOCSBRK: u32 = 0x5427;
pub const TIOCCBRK: u32 = 0x5428;
pub const TIOCMIWAIT: u32 = 0x545C;
pub const TIOCGICOUNT: u32 = 0x545D;

/// Converts `PinState` to Linux `TIOCM_*` bitmask
pub fn pin_state_to_tiocm(pins: PinState) -> u32 {
    let mut bits = 0u32;
    if pins.contains(PinState::RTS) {
        bits |= TIOCM_RTS;
    }
    if pins.contains(PinState::CTS) {
        bits |= TIOCM_CTS;
    }
    if pins.contains(PinState::DTR) {
        bits |= TIOCM_DTR;
    }
    if pins.contains(PinState::DSR) {
        bits |= TIOCM_DSR;
    }
    if pins.contains(PinState::DCD) {
        bits |= TIOCM_CAR;
    }
    if pins.contains(PinState::RI) {
        bits |= TIOCM_RNG;
    }
    bits
}

/// Converts Linux `TIOCM_*` bitmask to `PinState`
pub fn tiocm_to_pin_state(bits: u32) -> PinState {
    let mut pins = PinState::empty();
    if (bits & TIOCM_RTS) != 0 {
        pins |= PinState::RTS;
    }
    if (bits & TIOCM_CTS) != 0 {
        pins |= PinState::CTS;
    }
    if (bits & TIOCM_DTR) != 0 {
        pins |= PinState::DTR;
    }
    if (bits & TIOCM_DSR) != 0 {
        pins |= PinState::DSR;
    }
    if (bits & TIOCM_CAR) != 0 {
        pins |= PinState::DCD;
    }
    if (bits & TIOCM_RNG) != 0 {
        pins |= PinState::RI;
    }
    pins
}

/// Serial interrupt / delta event counters (compatible with Linux `struct serial_icounter_struct`)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SerialCounters {
    pub cts: u32,
    pub dsr: u32,
    pub rng: u32,
    pub dcd: u32,
    pub rx: u32,
    pub tx: u32,
    pub frame: u32,
    pub overrun: u32,
    pub parity: u32,
    pub brk: u32,
}

impl SerialCounters {
    /// Serializes counters to 40 bytes little-endian matching Linux kernel `serial_icounter_struct`
    pub fn to_bytes(&self) -> [u8; 40] {
        let mut buf = [0u8; 40];
        buf[0..4].copy_from_slice(&self.cts.to_ne_bytes());
        buf[4..8].copy_from_slice(&self.dsr.to_ne_bytes());
        buf[8..12].copy_from_slice(&self.rng.to_ne_bytes());
        buf[12..16].copy_from_slice(&self.dcd.to_ne_bytes());
        buf[16..20].copy_from_slice(&self.rx.to_ne_bytes());
        buf[20..24].copy_from_slice(&self.tx.to_ne_bytes());
        buf[24..28].copy_from_slice(&self.frame.to_ne_bytes());
        buf[28..32].copy_from_slice(&self.overrun.to_ne_bytes());
        buf[32..36].copy_from_slice(&self.parity.to_ne_bytes());
        buf[36..40].copy_from_slice(&self.brk.to_ne_bytes());
        buf
    }
}

/// Dispatches and processes serial ioctl commands for CUSE character device
pub struct SerialIoctlHandler {
    pins: PinState,
    config: SerialConfig,
    counters: SerialCounters,
    pin_broadcast_tx: broadcast::Sender<PinState>,
}

impl SerialIoctlHandler {
    pub fn new(initial_pins: PinState, config: SerialConfig) -> Self {
        let (pin_broadcast_tx, _) = broadcast::channel(64);
        Self {
            pins: initial_pins,
            config,
            counters: SerialCounters::default(),
            pin_broadcast_tx,
        }
    }

    pub fn pins(&self) -> PinState {
        self.pins
    }

    pub fn config(&self) -> SerialConfig {
        self.config
    }

    pub fn counters(&self) -> SerialCounters {
        self.counters
    }

    pub fn subscribe_pin_changes(&self) -> broadcast::Receiver<PinState> {
        self.pin_broadcast_tx.subscribe()
    }

    /// Sets modem pins and updates transition delta counters
    pub fn set_pins(&mut self, new_pins: PinState) -> bool {
        if self.pins == new_pins {
            return false;
        }

        let old = self.pins;
        self.pins = new_pins;

        // Record pin transitions
        if old.contains(PinState::CTS) != new_pins.contains(PinState::CTS) {
            self.counters.cts = self.counters.cts.saturating_add(1);
        }
        if old.contains(PinState::DSR) != new_pins.contains(PinState::DSR) {
            self.counters.dsr = self.counters.dsr.saturating_add(1);
        }
        if old.contains(PinState::RI) != new_pins.contains(PinState::RI) {
            self.counters.rng = self.counters.rng.saturating_add(1);
        }
        if old.contains(PinState::DCD) != new_pins.contains(PinState::DCD) {
            self.counters.dcd = self.counters.dcd.saturating_add(1);
        }
        if old.contains(PinState::BREAK) != new_pins.contains(PinState::BREAK)
            && new_pins.contains(PinState::BREAK)
        {
            self.counters.brk = self.counters.brk.saturating_add(1);
        }

        // Notify waiting tasks (e.g. TIOCMIWAIT)
        let _ = self.pin_broadcast_tx.send(new_pins);
        true
    }

    /// Sets or clears BREAK condition
    pub fn set_break(&mut self, active: bool) -> bool {
        let mut new_pins = self.pins;
        if active {
            new_pins |= PinState::BREAK;
        } else {
            new_pins &= !PinState::BREAK;
        }
        self.set_pins(new_pins)
    }

    /// Intercepts and executes a single serial ioctl
    pub fn handle_ioctl(&mut self, cmd: u32, arg_in: &[u8]) -> Result<Vec<u8>> {
        match cmd {
            TIOCMGET => {
                let bits = pin_state_to_tiocm(self.pins);
                Ok(bits.to_ne_bytes().to_vec())
            }
            TIOCMSET => {
                if arg_in.len() < 4 {
                    return Err(HalError::InvalidIoctlArg(
                        "TIOCMSET requires 4 bytes".into(),
                    ));
                }
                let bits = u32::from_ne_bytes([arg_in[0], arg_in[1], arg_in[2], arg_in[3]]);
                let mut new_pins = tiocm_to_pin_state(bits);
                // Retain BREAK status across TIOCMSET
                if self.pins.contains(PinState::BREAK) {
                    new_pins |= PinState::BREAK;
                }
                self.set_pins(new_pins);
                Ok(Vec::new())
            }
            TIOCMBIS => {
                if arg_in.len() < 4 {
                    return Err(HalError::InvalidIoctlArg(
                        "TIOCMBIS requires 4 bytes".into(),
                    ));
                }
                let bits = u32::from_ne_bytes([arg_in[0], arg_in[1], arg_in[2], arg_in[3]]);
                let to_set = tiocm_to_pin_state(bits);
                self.set_pins(self.pins | to_set);
                Ok(Vec::new())
            }
            TIOCMBIC => {
                if arg_in.len() < 4 {
                    return Err(HalError::InvalidIoctlArg(
                        "TIOCMBIC requires 4 bytes".into(),
                    ));
                }
                let bits = u32::from_ne_bytes([arg_in[0], arg_in[1], arg_in[2], arg_in[3]]);
                let to_clear = tiocm_to_pin_state(bits);
                self.set_pins(self.pins & !to_clear);
                Ok(Vec::new())
            }
            TIOCSBRK => {
                self.set_break(true);
                Ok(Vec::new())
            }
            TIOCCBRK => {
                self.set_break(false);
                Ok(Vec::new())
            }
            TCSBRK => {
                // Linux tty semantics: a non-zero argument means "wait until the
                // output has drained" and must NOT emit a break; only argument
                // 0 sends a break.
                let arg = if arg_in.len() >= 4 {
                    u32::from_ne_bytes([arg_in[0], arg_in[1], arg_in[2], arg_in[3]])
                } else {
                    0
                };
                if arg == 0 {
                    // Mock approximation of tcsendbreak(): pulse BREAK.
                    self.set_break(true);
                    self.set_break(false);
                }
                Ok(Vec::new())
            }
            TIOCGICOUNT => Ok(self.counters.to_bytes().to_vec()),
            TCGETS => {
                // Return serial config (baud, data, stop, parity)
                let mut out = [0u8; 8];
                out[0..4].copy_from_slice(&self.config.baud_rate.to_ne_bytes());
                out[4] = self.config.data_bits;
                out[5] = self.config.stop_bits;
                out[6] = self.config.parity;
                out[7] = 0;
                Ok(out.to_vec())
            }
            TCSETS => {
                if arg_in.len() < 8 {
                    return Err(HalError::InvalidIoctlArg(
                        "TCSETS requires at least 8 bytes".into(),
                    ));
                }
                let baud_rate = u32::from_ne_bytes([arg_in[0], arg_in[1], arg_in[2], arg_in[3]]);
                let data_bits = arg_in[4];
                let stop_bits = arg_in[5];
                let parity = arg_in[6];
                self.config = SerialConfig {
                    baud_rate,
                    data_bits,
                    stop_bits,
                    parity,
                };
                Ok(Vec::new())
            }
            other => Err(HalError::UnsupportedIoctl(other)),
        }
    }

    pub fn prepare_pin_wait(&self, tiocm_mask: u32) -> (broadcast::Receiver<PinState>, u32) {
        (
            self.subscribe_pin_changes(),
            pin_state_to_tiocm(self.pins) & tiocm_mask,
        )
    }

    /// Asynchronously awaits modem pin change matching `tiocm_mask` (implements `TIOCMIWAIT`)
    pub async fn wait_for_pin_change(
        &self,
        tiocm_mask: u32,
        timeout: Option<Duration>,
    ) -> Result<PinState> {
        let (rx, starting_tiocm) = self.prepare_pin_wait(tiocm_mask);
        await_pin_change(rx, starting_tiocm, tiocm_mask, timeout).await
    }
}

/// Helper function to await pin transition without holding mutex
pub async fn await_pin_change(
    mut rx: broadcast::Receiver<PinState>,
    starting_tiocm: u32,
    tiocm_mask: u32,
    timeout: Option<Duration>,
) -> Result<PinState> {
    let wait_fut = async {
        loop {
            match rx.recv().await {
                Ok(new_pins) => {
                    let new_tiocm = pin_state_to_tiocm(new_pins) & tiocm_mask;
                    if new_tiocm != starting_tiocm {
                        return Ok(new_pins);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(HalError::Io(std::io::Error::new(
                        std::io::ErrorKind::ConnectionAborted,
                        "pin monitor channel closed",
                    )));
                }
            }
        }
    };

    if let Some(dur) = timeout {
        match tokio::time::timeout(dur, wait_fut).await {
            Ok(res) => res,
            Err(_) => Err(HalError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "TIOCMIWAIT timed out",
            ))),
        }
    } else {
        wait_fut.await
    }
}

/// Tier-2 Linux CUSE Character Device Endpoint supporting full modem pin emulation
pub struct CuseEndpoint {
    id: EndpointId,
    name: String,
    device_name: String,
    handler: Arc<Mutex<SerialIoctlHandler>>,
    throttler: TokenBucketThrottler,
    master_stream: tokio::io::DuplexStream,
    mock_client: Option<tokio::io::DuplexStream>,
}

impl CuseEndpoint {
    /// Creates a new CUSE character device endpoint
    pub fn open(
        id: EndpointId,
        name: impl Into<String>,
        device_name: impl Into<String>,
        config: SerialConfig,
        initial_pins: PinState,
    ) -> Result<Self> {
        let (master_stream, mock_client) = tokio::io::duplex(65536);
        let throttler = TokenBucketThrottler::new(config);
        let handler = Arc::new(Mutex::new(SerialIoctlHandler::new(initial_pins, config)));

        Ok(Self {
            id,
            name: name.into(),
            device_name: device_name.into(),
            handler,
            throttler,
            master_stream,
            mock_client: Some(mock_client),
        })
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn handler(&self) -> &Arc<Mutex<SerialIoctlHandler>> {
        &self.handler
    }

    pub fn throttler(&self) -> &TokenBucketThrottler {
        &self.throttler
    }

    pub fn throttler_mut(&mut self) -> &mut TokenBucketThrottler {
        &mut self.throttler
    }

    /// Dispatches an ioctl command to the endpoint's handler.
    ///
    /// Line-rate changes made through the device node (`TCSETS`) are mirrored
    /// onto the software baud throttler; previously only the handler's config
    /// changed and writes kept using the old rate.
    pub async fn dispatch_ioctl(&mut self, cmd: u32, arg_in: &[u8]) -> Result<Vec<u8>> {
        let result = {
            let mut guard = self.handler.lock().await;
            guard.handle_ioctl(cmd, arg_in)
        };
        if result.is_ok() && cmd == TCSETS {
            let cfg = self.handler.lock().await.config();
            self.throttler.update_config(cfg);
        }
        result
    }

    /// Asynchronously awaits modem pin transition (implements `TIOCMIWAIT`)
    pub async fn wait_for_pin_change(
        &self,
        tiocm_mask: u32,
        timeout: Option<Duration>,
    ) -> Result<PinState> {
        let (rx, starting_tiocm) = {
            let guard = self.handler.lock().await;
            guard.prepare_pin_wait(tiocm_mask)
        };
        await_pin_change(rx, starting_tiocm, tiocm_mask, timeout).await
    }

    /// Extracts mock client duplex stream for unit and integration testing
    pub fn take_mock_client(&mut self) -> Option<tokio::io::DuplexStream> {
        self.mock_client.take()
    }
}

impl AsyncEndpoint for CuseEndpoint {
    fn id(&self) -> EndpointId {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn recv(&mut self) -> Pin<Box<dyn Future<Output = std::io::Result<SerialEvent>> + Send + '_>> {
        Box::pin(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = [0u8; 4096];
            let n = self.master_stream.read(&mut buf).await?;

            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "CUSE client disconnected",
                ));
            }

            let now_ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;

            let current_pins = self.handler.lock().await.pins();

            Ok(SerialEvent {
                timestamp_ns: now_ns,
                source_id: self.id,
                flags: FrameFlags::empty(),
                pins: current_pins,
                payload: Bytes::copy_from_slice(&buf[..n]),
            })
        })
    }

    fn send(
        &mut self,
        event: SerialEvent,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send + '_>> {
        Box::pin(async move {
            use tokio::io::AsyncWriteExt;
            if !event.payload.is_empty() {
                // Apply software token-bucket baud rate throttling
                self.throttler.throttle_async(event.payload.len()).await;
                self.master_stream.write_all(&event.payload).await?;
            }
            Ok(())
        })
    }

    fn set_pins(&mut self, pins: PinState) -> std::io::Result<()> {
        let handler = Arc::clone(&self.handler);
        tokio::spawn(async move {
            let mut guard = handler.lock().await;
            guard.set_pins(pins);
        });
        Ok(())
    }

    fn configure(&mut self, cfg: SerialConfig) -> std::io::Result<()> {
        self.throttler.update_config(cfg);
        let handler = Arc::clone(&self.handler);
        tokio::spawn(async move {
            let mut guard = handler.lock().await;
            guard.config = cfg;
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn test_cuse_ioctl_modem_get_set() {
        let mut handler = SerialIoctlHandler::new(PinState::empty(), SerialConfig::default());

        // Initial TIOCMGET should be 0
        let res = handler.handle_ioctl(TIOCMGET, &[]).unwrap();
        let bits = u32::from_ne_bytes([res[0], res[1], res[2], res[3]]);
        assert_eq!(bits, 0);

        // TIOCMSET DTR | RTS
        let set_bits = (TIOCM_DTR | TIOCM_RTS).to_ne_bytes();
        handler.handle_ioctl(TIOCMSET, &set_bits).unwrap();

        assert!(handler.pins().contains(PinState::DTR));
        assert!(handler.pins().contains(PinState::RTS));
        assert!(!handler.pins().contains(PinState::CTS));

        // TIOCMGET returns updated bits
        let res2 = handler.handle_ioctl(TIOCMGET, &[]).unwrap();
        let bits2 = u32::from_ne_bytes([res2[0], res2[1], res2[2], res2[3]]);
        assert_eq!(bits2, TIOCM_DTR | TIOCM_RTS);

        // TIOCMBIS CTS
        let bis_bits = TIOCM_CTS.to_ne_bytes();
        handler.handle_ioctl(TIOCMBIS, &bis_bits).unwrap();
        assert!(handler.pins().contains(PinState::CTS));

        // TIOCMBIC RTS
        let bic_bits = TIOCM_RTS.to_ne_bytes();
        handler.handle_ioctl(TIOCMBIC, &bic_bits).unwrap();
        assert!(!handler.pins().contains(PinState::RTS));
        assert!(handler.pins().contains(PinState::DTR));
        assert!(handler.pins().contains(PinState::CTS));
    }

    #[tokio::test]
    async fn test_cuse_break_and_counters() {
        let mut handler = SerialIoctlHandler::new(PinState::empty(), SerialConfig::default());

        // TIOCSBRK sets break
        handler.handle_ioctl(TIOCSBRK, &[]).unwrap();
        assert!(handler.pins().contains(PinState::BREAK));
        assert_eq!(handler.counters().brk, 1);

        // TIOCCBRK clears break
        handler.handle_ioctl(TIOCCBRK, &[]).unwrap();
        assert!(!handler.pins().contains(PinState::BREAK));
        assert_eq!(handler.counters().brk, 1);

        // TCSBRK triggers break toggle
        handler.handle_ioctl(TCSBRK, &[]).unwrap();
        assert_eq!(handler.counters().brk, 2);
    }

    #[tokio::test]
    async fn test_cuse_tcsbrk_nonzero_arg_does_not_emit_break() {
        let mut handler = SerialIoctlHandler::new(PinState::empty(), SerialConfig::default());

        // TCSBRK with a non-zero argument means "drain output", not "send break".
        handler
            .handle_ioctl(TCSBRK, &1u32.to_ne_bytes())
            .expect("TCSBRK drain");
        assert_eq!(handler.counters().brk, 0);
        assert!(!handler.pins().contains(PinState::BREAK));

        // TCSBRK with argument 0 sends a break (approximated as a pulse).
        handler
            .handle_ioctl(TCSBRK, &0u32.to_ne_bytes())
            .expect("TCSBRK break");
        assert_eq!(handler.counters().brk, 1);
        assert!(!handler.pins().contains(PinState::BREAK));
    }

    #[tokio::test]
    async fn test_cuse_tcsets_reconfigures_throttler() {
        let config = SerialConfig {
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
        };
        let mut ep = CuseEndpoint::open(9, "cuse_baud", "ttyOmniBaud", config, PinState::empty())
            .expect("open cuse endpoint");
        assert_eq!(ep.throttler().byte_rate(), 11_520.0);

        let mut arg = [0u8; 8];
        arg[0..4].copy_from_slice(&9_600u32.to_ne_bytes());
        arg[4] = 8;
        arg[5] = 1;
        arg[6] = 0;
        ep.dispatch_ioctl(TCSETS, &arg).await.expect("TCSETS");

        assert_eq!(ep.handler().lock().await.config().baud_rate, 9_600);
        assert_eq!(ep.throttler().config().baud_rate, 9_600);
        assert_eq!(ep.throttler().byte_rate(), 960.0);
    }

    #[tokio::test]
    async fn test_cuse_tiocmiwait_wakeup() {
        let ep = CuseEndpoint::open(
            1,
            "cuse_wait",
            "ttyOmniWait",
            SerialConfig::default(),
            PinState::empty(),
        )
        .expect("open cuse ep");

        let ep_arc = Arc::new(ep);
        let ep_clone = Arc::clone(&ep_arc);

        let wait_handle = tokio::spawn(async move {
            ep_clone
                .wait_for_pin_change(TIOCM_CTS, Some(Duration::from_millis(500)))
                .await
        });

        // Give the task time to yield
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Transition CTS on endpoint handler
        {
            let mut guard = ep_arc.handler().lock().await;
            guard.set_pins(PinState::CTS);
        }

        let result = wait_handle.await.unwrap().expect("wait success");
        assert!(result.contains(PinState::CTS));
        assert_eq!(ep_arc.handler().lock().await.counters().cts, 1);
    }

    #[tokio::test]
    async fn test_cuse_endpoint_io_and_throttling() {
        let config = SerialConfig {
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
        };

        let mut ep = CuseEndpoint::open(
            5,
            "cuse0",
            "ttyOmni0",
            config,
            PinState::DTR | PinState::RTS,
        )
        .expect("open cuse endpoint");

        assert_eq!(ep.id(), 5);
        assert_eq!(ep.name(), "cuse0");
        assert_eq!(ep.device_name(), "ttyOmni0");

        let mut client = ep.take_mock_client().expect("take client");

        // Client writes to device -> ep.recv()
        client
            .write_all(b"HELLO_CUSE\n")
            .await
            .expect("client write");

        let event = ep.recv().await.expect("ep recv");
        assert_eq!(event.source_id, 5);
        assert_eq!(&event.payload[..], b"HELLO_CUSE\n");
        assert_eq!(event.pins, PinState::DTR | PinState::RTS);

        // ep.send() -> Client reads
        let send_evt = SerialEvent {
            timestamp_ns: 1234,
            source_id: 5,
            flags: FrameFlags::empty(),
            pins: PinState::empty(),
            payload: Bytes::from_static(b"REPLY_CUSE\n"),
        };
        ep.send(send_evt).await.expect("ep send");

        let mut buf = vec![0u8; 11];
        client.read_exact(&mut buf).await.expect("client read");
        assert_eq!(&buf[..], b"REPLY_CUSE\n");
    }
}
