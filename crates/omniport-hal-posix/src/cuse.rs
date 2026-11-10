use crate::error::{HalError, Result};
use bytes::Bytes;
use omniport_core::TokenBucketThrottler;
use omniport_core::{AsyncEndpoint, EndpointId, FrameFlags, PinState, SerialConfig, SerialEvent};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::{Mutex, broadcast};
#[cfg(target_os = "linux")]
use tokio::sync::{Notify, oneshot};

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

// Linux Serial Ioctl Command Constants (<asm-generic/ioctls.h>)
pub const TCGETS: u32 = 0x5401;
pub const TCSETS: u32 = 0x5402;
pub const TCSETSW: u32 = 0x5403;
pub const TCSETSF: u32 = 0x5404;
pub const TCSBRK: u32 = 0x5409;
pub const TIOCMGET: u32 = 0x5415;
pub const TIOCMBIS: u32 = 0x5416;
pub const TIOCMBIC: u32 = 0x5417;
pub const TIOCMSET: u32 = 0x5418;
pub const TIOCSBRK: u32 = 0x5427;
pub const TIOCCBRK: u32 = 0x5428;
pub const TCGETS2: u32 = 0x802C_542A;
pub const TCSETS2: u32 = 0x402C_542B;
pub const TCSETSW2: u32 = 0x402C_542C;
pub const TCSETSF2: u32 = 0x402C_542D;
pub const TIOCMIWAIT: u32 = 0x545C;
pub const TIOCGICOUNT: u32 = 0x545D;

// Linux UAPI <asm-generic/termbits.h> & <uapi/linux/serial.h> struct sizes
pub const KERNEL_TERMIOS_SIZE: usize = 36;
pub const KERNEL_TERMIOS2_SIZE: usize = 44;
pub const SERIAL_ICOUNTER_SIZE: usize = 80;
const KERNEL_NCCS: usize = 19;

// <asm-generic/termbits.h> c_cflag masks & standard speeds (shared across platforms for testing)
const K_CBAUD: u32 = 0o010017;
const K_BOTHER: u32 = 0o010000;
const K_CSIZE: u32 = 0x0000_0030;
const K_CS5: u32 = 0x0000_0000;
const K_CS6: u32 = 0x0000_0010;
const K_CS7: u32 = 0x0000_0020;
const K_CS8: u32 = 0x0000_0030;
const K_CSTOPB: u32 = 0x0000_0040;
const K_CREAD: u32 = 0x0000_0080;
const K_PARENB: u32 = 0x0000_0100;
const K_PARODD: u32 = 0x0000_0200;
const K_CLOCAL: u32 = 0x0000_0800;

// Linux <uapi/linux/fuse.h> FUSE/CUSE ABI constants
const FUSE_KERNEL_VERSION: u32 = 7;
const FUSE_KERNEL_MINOR_VERSION: u32 = 31;
const CUSE_UNRESTRICTED_IOCTL: u32 = 1 << 0;

const FUSE_OPEN: u32 = 14;
const FUSE_READ: u32 = 15;
const FUSE_WRITE: u32 = 16;
const FUSE_RELEASE: u32 = 18;
const FUSE_FLUSH: u32 = 25;
const FUSE_INTERRUPT: u32 = 36;
const FUSE_DESTROY: u32 = 38;
const FUSE_IOCTL: u32 = 39;
const FUSE_POLL: u32 = 40;
const CUSE_INIT: u32 = 4096;

const FOPEN_DIRECT_IO: u32 = 1 << 0;
const FOPEN_NONSEEKABLE: u32 = 1 << 2;

const FUSE_IOCTL_UNRESTRICTED: u32 = 1 << 1;
const FUSE_IOCTL_RETRY: u32 = 1 << 2;

const FUSE_POLL_SCHEDULE_NOTIFY: u32 = 1 << 0;
#[cfg(target_os = "linux")]
const FUSE_NOTIFY_POLL: i32 = 1;

const LINUX_POLLIN: u32 = 0x0001;
const LINUX_POLLOUT: u32 = 0x0004;

#[cfg(target_os = "linux")]
const LINUX_EINTR: i32 = 4;
const LINUX_EIO: i32 = 5;
const LINUX_EAGAIN: i32 = 11;
const LINUX_EFAULT: i32 = 14;
const LINUX_EINVAL: i32 = 22;
const LINUX_ENOTTY: i32 = 25;
const LINUX_ENOSYS: i32 = 38;

const FUSE_IN_HEADER_LEN: usize = 40;
const FUSE_OUT_HEADER_LEN: usize = 16;
const CUSE_INIT_IN_LEN: usize = 16;
const CUSE_INIT_OUT_LEN: usize = 72;
const FUSE_OPEN_OUT_LEN: usize = 16;
const FUSE_READ_IN_LEN: usize = 40;
const FUSE_WRITE_IN_LEN: usize = 40;
const FUSE_WRITE_OUT_LEN: usize = 8;
const FUSE_IOCTL_IN_LEN: usize = 32;
const FUSE_IOCTL_OUT_LEN: usize = 16;
const FUSE_POLL_IN_LEN: usize = 24;
const FUSE_POLL_OUT_LEN: usize = 8;
const FUSE_MAX_XFER: u32 = 131_072;
#[cfg(target_os = "linux")]
const CUSE_READ_BUFFER_LEN: usize = (FUSE_MAX_XFER as usize) + 4096;

// Linux O_NONBLOCK on x86/arm64/riscv (`0o4000`); use `libc::O_NONBLOCK` directly when building on Linux.
#[cfg(target_os = "linux")]
const LINUX_O_NONBLOCK: u32 = libc::O_NONBLOCK as u32;
#[cfg(not(target_os = "linux"))]
const LINUX_O_NONBLOCK: u32 = 0x0800;

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

const STANDARD_BAUDS: &[(u32, u32)] = &[
    (0o000000, 0),
    (0o000001, 50),
    (0o000002, 75),
    (0o000003, 110),
    (0o000004, 134),
    (0o000005, 150),
    (0o000006, 200),
    (0o000007, 300),
    (0o000010, 600),
    (0o000011, 1_200),
    (0o000012, 1_800),
    (0o000013, 2_400),
    (0o000014, 4_800),
    (0o000015, 9_600),
    (0o000016, 19_200),
    (0o000017, 38_400),
    (0o010001, 57_600),
    (0o010002, 115_200),
    (0o010003, 230_400),
    (0o010004, 460_800),
    (0o010005, 500_000),
    (0o010006, 576_000),
    (0o010007, 921_600),
    (0o010010, 1_000_000),
    (0o010011, 1_152_000),
    (0o010012, 1_500_000),
    (0o010013, 2_000_000),
    (0o010014, 2_500_000),
    (0o010015, 3_000_000),
    (0o010016, 3_500_000),
    (0o010017, 4_000_000),
];

fn encode_kbaud(baud: u32) -> u32 {
    STANDARD_BAUDS
        .iter()
        .find_map(|&(code, b)| if b == baud { Some(code) } else { None })
        .unwrap_or(K_BOTHER)
}

fn decode_kbaud(code: u32) -> Option<u32> {
    STANDARD_BAUDS
        .iter()
        .find_map(|&(c, b)| if c == code { Some(b) } else { None })
}

/// Linux kernel UAPI `struct termios` (36 bytes, `<asm-generic/termbits.h>`), used by
/// `TCGETS` / `TCSETS` (`termios2` extends it to 44 bytes with `c_ispeed` and `c_ospeed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KernelTermios {
    pub c_iflag: u32,
    pub c_oflag: u32,
    pub c_cflag: u32,
    pub c_lflag: u32,
    pub c_line: u8,
    pub c_cc: [u8; KERNEL_NCCS],
}

impl KernelTermios {
    pub fn from_config(cfg: &SerialConfig) -> Self {
        let mut t = Self {
            c_iflag: 0,
            c_oflag: 0,
            c_cflag: K_CREAD | K_CLOCAL,
            c_lflag: 0,
            c_line: 0,
            c_cc: [0u8; KERNEL_NCCS],
        };
        t.c_cc[6] = 1; // VMIN = 1 (raw blocking single-byte threshold)
        t.sync_from_config(cfg);
        t
    }

    pub fn sync_from_config(&mut self, cfg: &SerialConfig) {
        self.c_cflag &= !(K_CBAUD | K_CSIZE | K_CSTOPB | K_PARENB | K_PARODD);
        self.c_cflag |= K_CREAD | K_CLOCAL | encode_kbaud(cfg.baud_rate);
        self.c_cflag |= match cfg.data_bits {
            5 => K_CS5,
            6 => K_CS6,
            7 => K_CS7,
            _ => K_CS8,
        };
        if cfg.stop_bits >= 2 {
            self.c_cflag |= K_CSTOPB;
        }
        match cfg.parity {
            1 => self.c_cflag |= K_PARENB | K_PARODD,
            2 => self.c_cflag |= K_PARENB,
            _ => {}
        }
    }

    pub fn decode_config(
        &self,
        fallback_baud: u32,
        speeds: Option<(u32, u32)>,
    ) -> Result<SerialConfig> {
        let baud_bits = self.c_cflag & K_CBAUD;
        let baud_rate = if baud_bits == K_BOTHER {
            speeds
                .map(|(_, ospeed)| ospeed)
                .filter(|&b| b > 0)
                .unwrap_or(fallback_baud)
        } else {
            match decode_kbaud(baud_bits) {
                Some(0) | None => {
                    if let Some((_, ospeed)) = speeds
                        && ospeed > 0
                    {
                        ospeed
                    } else {
                        fallback_baud
                    }
                }
                Some(b) => b,
            }
        };

        let data_bits = match self.c_cflag & K_CSIZE {
            K_CS5 => 5,
            K_CS6 => 6,
            K_CS7 => 7,
            _ => 8,
        };
        let stop_bits = if (self.c_cflag & K_CSTOPB) != 0 { 2 } else { 1 };
        let parity = if (self.c_cflag & K_PARENB) != 0 {
            if (self.c_cflag & K_PARODD) != 0 { 1 } else { 2 }
        } else {
            0
        };

        Ok(SerialConfig {
            baud_rate,
            data_bits,
            stop_bits,
            parity,
        })
    }

    pub fn to_bytes(&self) -> [u8; KERNEL_TERMIOS_SIZE] {
        let mut buf = [0u8; KERNEL_TERMIOS_SIZE];
        buf[0..4].copy_from_slice(&self.c_iflag.to_ne_bytes());
        buf[4..8].copy_from_slice(&self.c_oflag.to_ne_bytes());
        buf[8..12].copy_from_slice(&self.c_cflag.to_ne_bytes());
        buf[12..16].copy_from_slice(&self.c_lflag.to_ne_bytes());
        buf[16] = self.c_line;
        buf[17..36].copy_from_slice(&self.c_cc);
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        if buf.len() < KERNEL_TERMIOS_SIZE {
            return Err(HalError::InvalidIoctlArg(format!(
                "kernel struct termios requires {KERNEL_TERMIOS_SIZE} bytes, got {}",
                buf.len()
            )));
        }
        let mut c_cc = [0u8; KERNEL_NCCS];
        c_cc.copy_from_slice(&buf[17..36]);
        Ok(Self {
            c_iflag: u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]),
            c_oflag: u32::from_ne_bytes([buf[4], buf[5], buf[6], buf[7]]),
            c_cflag: u32::from_ne_bytes([buf[8], buf[9], buf[10], buf[11]]),
            c_lflag: u32::from_ne_bytes([buf[12], buf[13], buf[14], buf[15]]),
            c_line: buf[16],
            c_cc,
        })
    }

    pub fn to_termios2_bytes(&self, baud_rate: u32) -> [u8; KERNEL_TERMIOS2_SIZE] {
        let mut buf = [0u8; KERNEL_TERMIOS2_SIZE];
        buf[0..36].copy_from_slice(&self.to_bytes());
        buf[36..40].copy_from_slice(&baud_rate.to_ne_bytes());
        buf[40..44].copy_from_slice(&baud_rate.to_ne_bytes());
        buf
    }

    pub fn from_termios2_bytes(buf: &[u8]) -> Result<(Self, u32, u32)> {
        if buf.len() < KERNEL_TERMIOS2_SIZE {
            return Err(HalError::InvalidIoctlArg(format!(
                "kernel struct termios2 requires {KERNEL_TERMIOS2_SIZE} bytes, got {}",
                buf.len()
            )));
        }
        let base = Self::from_bytes(&buf[..KERNEL_TERMIOS_SIZE])?;
        let ispeed = u32::from_ne_bytes([buf[36], buf[37], buf[38], buf[39]]);
        let ospeed = u32::from_ne_bytes([buf[40], buf[41], buf[42], buf[43]]);
        Ok((base, ispeed, ospeed))
    }
}

/// Serial interrupt / delta event counters (80-byte Linux `<uapi/linux/serial.h>`
/// `struct serial_icounter_struct`: 11 counters + 9 reserved `i32`s)
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
    pub buf_overrun: u32,
}

impl SerialCounters {
    /// Serializes counters to 80 bytes native-endian matching Linux kernel `serial_icounter_struct`
    pub fn to_bytes(&self) -> [u8; SERIAL_ICOUNTER_SIZE] {
        let mut buf = [0u8; SERIAL_ICOUNTER_SIZE];
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
        buf[40..44].copy_from_slice(&self.buf_overrun.to_ne_bytes());
        // Bytes 44..80 remain zero (`int reserved[9]`).
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        if buf.len() < SERIAL_ICOUNTER_SIZE {
            return Err(HalError::InvalidIoctlArg(format!(
                "serial_icounter_struct requires {SERIAL_ICOUNTER_SIZE} bytes, got {}",
                buf.len()
            )));
        }
        let word =
            |off: usize| u32::from_ne_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        Ok(Self {
            cts: word(0),
            dsr: word(4),
            rng: word(8),
            dcd: word(12),
            rx: word(16),
            tx: word(20),
            frame: word(24),
            overrun: word(28),
            parity: word(32),
            brk: word(36),
            buf_overrun: word(40),
        })
    }
}

/// Dispatches and processes serial ioctl commands for CUSE character device
pub struct SerialIoctlHandler {
    pins: PinState,
    config: SerialConfig,
    termios: KernelTermios,
    counters: SerialCounters,
    pin_broadcast_tx: broadcast::Sender<PinState>,
}

impl SerialIoctlHandler {
    pub fn new(initial_pins: PinState, config: SerialConfig) -> Self {
        let (pin_broadcast_tx, _) = broadcast::channel(64);
        Self {
            pins: initial_pins,
            config,
            termios: KernelTermios::from_config(&config),
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

    pub fn termios(&self) -> KernelTermios {
        self.termios
    }

    pub fn counters(&self) -> SerialCounters {
        self.counters
    }

    pub fn set_config(&mut self, config: SerialConfig) {
        self.config = config;
        self.termios.sync_from_config(&config);
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
                // Linux tty semantics: a non-zero argument drains output and must not emit a break.
                let arg = if arg_in.len() >= 4 {
                    u32::from_ne_bytes([arg_in[0], arg_in[1], arg_in[2], arg_in[3]])
                } else {
                    0
                };
                if arg == 0 {
                    self.set_break(true);
                    self.set_break(false);
                }
                Ok(Vec::new())
            }
            TIOCGICOUNT => Ok(self.counters.to_bytes().to_vec()),
            TCGETS => {
                self.termios.sync_from_config(&self.config);
                Ok(self.termios.to_bytes().to_vec())
            }
            TCSETS | TCSETSW | TCSETSF => {
                let parsed = KernelTermios::from_bytes(arg_in)?;
                let cfg = parsed.decode_config(self.config.baud_rate, None)?;
                self.termios = parsed;
                self.config = cfg;
                Ok(Vec::new())
            }
            TCGETS2 => {
                self.termios.sync_from_config(&self.config);
                Ok(self
                    .termios
                    .to_termios2_bytes(self.config.baud_rate)
                    .to_vec())
            }
            TCSETS2 | TCSETSW2 | TCSETSF2 => {
                let (parsed, ispeed, ospeed) = KernelTermios::from_termios2_bytes(arg_in)?;
                let cfg = parsed.decode_config(self.config.baud_rate, Some((ispeed, ospeed)))?;
                self.termios = parsed;
                self.config = cfg;
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

fn is_termios_set_ioctl(cmd: u32) -> bool {
    matches!(
        cmd,
        TCSETS | TCSETSW | TCSETSF | TCSETS2 | TCSETSW2 | TCSETSF2
    )
}

fn ioctl_retry_sizes(cmd: u32) -> Option<(usize, usize)> {
    match cmd {
        TCGETS => Some((0, KERNEL_TERMIOS_SIZE)),
        TCSETS | TCSETSW | TCSETSF => Some((KERNEL_TERMIOS_SIZE, 0)),
        TCGETS2 => Some((0, KERNEL_TERMIOS2_SIZE)),
        TCSETS2 | TCSETSW2 | TCSETSF2 => Some((KERNEL_TERMIOS2_SIZE, 0)),
        TIOCMGET => Some((0, 4)),
        TIOCMSET | TIOCMBIS | TIOCMBIC => Some((4, 0)),
        TIOCGICOUNT => Some((0, SERIAL_ICOUNTER_SIZE)),
        TIOCSBRK | TIOCCBRK | TCSBRK | TIOCMIWAIT => Some((0, 0)),
        _ => None,
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

/// Decision produced for an inbound `/dev/cuse` FUSE request frame.
#[derive(Debug, PartialEq, Eq)]
pub enum FuseAction {
    /// Immediate response frame ready to write back to `/dev/cuse`.
    Reply(Vec<u8>),
    /// Blocking `FUSE_READ` waiting until TX bytes arrive in `tx_queue`.
    ParkRead { unique: u64, size: usize },
    /// Blocking `TIOCMIWAIT` waiting until modem pin matching `tiocm_mask` flips.
    ParkPinWait { unique: u64, tiocm_mask: u32 },
    /// `FUSE_INTERRUPT`: cancel the pending request whose `unique` ID is specified.
    Interrupt { target_unique: u64 },
    /// `FUSE_DESTROY`: acknowledge teardown and exit loop.
    Shutdown(Vec<u8>),
}

pub fn encode_fuse_reply(unique: u64, error: i32, payload: &[u8]) -> Vec<u8> {
    let total_len = (FUSE_OUT_HEADER_LEN + payload.len()) as u32;
    let mut out = Vec::with_capacity(total_len as usize);
    out.extend_from_slice(&total_len.to_ne_bytes());
    out.extend_from_slice(&error.to_ne_bytes());
    out.extend_from_slice(&unique.to_ne_bytes());
    out.extend_from_slice(payload);
    out
}

#[cfg(target_os = "linux")]
fn encode_fuse_notify_poll(kh: u64) -> Vec<u8> {
    // Asynchronous FUSE notification: unique == 0, error == notify code (1 = FUSE_NOTIFY_POLL),
    // followed by struct fuse_notify_poll_wakeup_out { u64 kh }.
    encode_fuse_reply(0, FUSE_NOTIFY_POLL, &kh.to_ne_bytes())
}

fn read_u32_ne(buf: &[u8], off: usize) -> u32 {
    u32::from_ne_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

fn read_u64_ne(buf: &[u8], off: usize) -> u64 {
    u64::from_ne_bytes([
        buf[off],
        buf[off + 1],
        buf[off + 2],
        buf[off + 3],
        buf[off + 4],
        buf[off + 5],
        buf[off + 6],
        buf[off + 7],
    ])
}

/// FUSE/CUSE binary protocol state machine shared between the Linux `/dev/cuse` loop and
/// platform-independent wire protocol unit tests.
pub struct CuseFuseSession {
    device_name: String,
    handler: Arc<Mutex<SerialIoctlHandler>>,
    config_tx: tokio::sync::watch::Sender<SerialConfig>,
    inbound_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    tx_queue: Arc<StdMutex<VecDeque<u8>>>,
    next_fh: u64,
    initialized: bool,
    poll_waiters: Arc<StdMutex<Vec<u64>>>,
}

impl CuseFuseSession {
    pub fn new(
        device_name: String,
        handler: Arc<Mutex<SerialIoctlHandler>>,
        config_tx: tokio::sync::watch::Sender<SerialConfig>,
        inbound_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
        tx_queue: Arc<StdMutex<VecDeque<u8>>>,
        poll_waiters: Arc<StdMutex<Vec<u64>>>,
    ) -> Self {
        Self {
            device_name,
            handler,
            config_tx,
            inbound_tx,
            tx_queue,
            next_fh: 1,
            initialized: false,
            poll_waiters,
        }
    }

    pub fn try_pop_tx(&self, max_len: usize) -> Option<Vec<u8>> {
        let mut q = self.tx_queue.lock().unwrap_or_else(|e| e.into_inner());
        if q.is_empty() {
            return None;
        }
        let n = q.len().min(max_len);
        let chunk: Vec<u8> = q.drain(..n).collect();
        Some(chunk)
    }

    /// Parses and executes one complete request packet from `/dev/cuse`.
    pub async fn process_packet(&mut self, packet: &[u8]) -> FuseAction {
        if packet.len() < FUSE_IN_HEADER_LEN {
            return FuseAction::Reply(encode_fuse_reply(0, -LINUX_EINVAL, &[]));
        }

        let total_len = read_u32_ne(packet, 0) as usize;
        let opcode = read_u32_ne(packet, 4);
        let unique = read_u64_ne(packet, 8);
        if total_len < FUSE_IN_HEADER_LEN || packet.len() < total_len {
            return FuseAction::Reply(encode_fuse_reply(unique, -LINUX_EINVAL, &[]));
        }
        let body = &packet[FUSE_IN_HEADER_LEN..total_len];

        match opcode {
            CUSE_INIT => {
                if body.len() < CUSE_INIT_IN_LEN {
                    return FuseAction::Reply(encode_fuse_reply(unique, -LINUX_EINVAL, &[]));
                }
                let in_major = read_u32_ne(body, 0);
                let in_minor = read_u32_ne(body, 4);
                let in_flags = read_u32_ne(body, 12);
                if in_major != FUSE_KERNEL_VERSION {
                    return FuseAction::Reply(encode_fuse_reply(unique, -LINUX_EIO, &[]));
                }

                self.initialized = true;
                let mut payload =
                    Vec::with_capacity(CUSE_INIT_OUT_LEN + self.device_name.len() + 9);
                payload.extend_from_slice(&FUSE_KERNEL_VERSION.to_ne_bytes());
                payload.extend_from_slice(&in_minor.min(FUSE_KERNEL_MINOR_VERSION).to_ne_bytes());
                payload.extend_from_slice(&0u32.to_ne_bytes()); // unused
                payload.extend_from_slice(&(in_flags & CUSE_UNRESTRICTED_IOCTL).to_ne_bytes());
                payload.extend_from_slice(&FUSE_MAX_XFER.to_ne_bytes()); // max_read
                payload.extend_from_slice(&FUSE_MAX_XFER.to_ne_bytes()); // max_write
                payload.extend_from_slice(&0u32.to_ne_bytes()); // dev_major (kernel allocated)
                payload.extend_from_slice(&0u32.to_ne_bytes()); // dev_minor (kernel allocated)
                payload.resize(CUSE_INIT_OUT_LEN, 0);
                payload.extend_from_slice(b"DEVNAME=");
                payload.extend_from_slice(self.device_name.as_bytes());
                payload.push(0);

                FuseAction::Reply(encode_fuse_reply(unique, 0, &payload))
            }
            FUSE_OPEN => {
                let fh = self.next_fh;
                self.next_fh = self.next_fh.wrapping_add(1).max(1);
                let mut payload = [0u8; FUSE_OPEN_OUT_LEN];
                payload[0..8].copy_from_slice(&fh.to_ne_bytes());
                payload[8..12]
                    .copy_from_slice(&(FOPEN_DIRECT_IO | FOPEN_NONSEEKABLE).to_ne_bytes());
                FuseAction::Reply(encode_fuse_reply(unique, 0, &payload))
            }
            FUSE_FLUSH | FUSE_RELEASE => FuseAction::Reply(encode_fuse_reply(unique, 0, &[])),
            FUSE_DESTROY => FuseAction::Shutdown(encode_fuse_reply(unique, 0, &[])),
            FUSE_INTERRUPT => {
                let target_unique = if body.len() >= 8 {
                    read_u64_ne(body, 0)
                } else {
                    0
                };
                FuseAction::Interrupt { target_unique }
            }
            FUSE_READ => {
                if body.len() < FUSE_READ_IN_LEN {
                    return FuseAction::Reply(encode_fuse_reply(unique, -LINUX_EINVAL, &[]));
                }
                let size = read_u32_ne(body, 16) as usize;
                let flags = read_u32_ne(body, 32);
                if let Some(chunk) = self.try_pop_tx(size) {
                    let mut guard = self.handler.lock().await;
                    guard.counters.rx = guard.counters.rx.saturating_add(chunk.len() as u32);
                    return FuseAction::Reply(encode_fuse_reply(unique, 0, &chunk));
                }
                if (flags & LINUX_O_NONBLOCK) != 0 {
                    FuseAction::Reply(encode_fuse_reply(unique, -LINUX_EAGAIN, &[]))
                } else {
                    FuseAction::ParkRead { unique, size }
                }
            }
            FUSE_WRITE => {
                if body.len() < FUSE_WRITE_IN_LEN {
                    return FuseAction::Reply(encode_fuse_reply(unique, -LINUX_EINVAL, &[]));
                }
                let size = read_u32_ne(body, 16) as usize;
                let data_start = FUSE_WRITE_IN_LEN;
                let Some(data_end) = data_start.checked_add(size) else {
                    return FuseAction::Reply(encode_fuse_reply(unique, -LINUX_EINVAL, &[]));
                };
                if body.len() < data_end {
                    return FuseAction::Reply(encode_fuse_reply(unique, -LINUX_EINVAL, &[]));
                }
                let chunk = body[data_start..data_end].to_vec();
                let written = chunk.len() as u32;
                if self.inbound_tx.send(chunk).await.is_err() {
                    return FuseAction::Reply(encode_fuse_reply(unique, -LINUX_EIO, &[]));
                }
                {
                    let mut guard = self.handler.lock().await;
                    guard.counters.tx = guard.counters.tx.saturating_add(written);
                }
                let mut payload = [0u8; FUSE_WRITE_OUT_LEN];
                payload[0..4].copy_from_slice(&written.to_ne_bytes());
                FuseAction::Reply(encode_fuse_reply(unique, 0, &payload))
            }
            FUSE_POLL => {
                if body.len() < FUSE_POLL_IN_LEN {
                    return FuseAction::Reply(encode_fuse_reply(unique, -LINUX_EINVAL, &[]));
                }
                let kh = read_u64_ne(body, 8);
                let flags = read_u32_ne(body, 16);
                let events = read_u32_ne(body, 20);

                let has_tx = !self
                    .tx_queue
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .is_empty();
                let mut revents = LINUX_POLLOUT;
                if has_tx {
                    revents |= LINUX_POLLIN;
                } else if (flags & FUSE_POLL_SCHEDULE_NOTIFY) != 0 && (events & LINUX_POLLIN) != 0 {
                    let mut waiters = self.poll_waiters.lock().unwrap_or_else(|e| e.into_inner());
                    if !waiters.contains(&kh) {
                        waiters.push(kh);
                    }
                }
                revents &= events;

                let mut payload = [0u8; FUSE_POLL_OUT_LEN];
                payload[0..4].copy_from_slice(&revents.to_ne_bytes());
                FuseAction::Reply(encode_fuse_reply(unique, 0, &payload))
            }
            FUSE_IOCTL => {
                if body.len() < FUSE_IOCTL_IN_LEN {
                    return FuseAction::Reply(encode_fuse_reply(unique, -LINUX_EINVAL, &[]));
                }
                let flags = read_u32_ne(body, 8);
                let cmd = read_u32_ne(body, 12);
                let arg = read_u64_ne(body, 16);
                let in_size = read_u32_ne(body, 24) as usize;
                let out_size = read_u32_ne(body, 28) as usize;
                let arg_in = &body[FUSE_IOCTL_IN_LEN..];
                if arg_in.len() < in_size {
                    return FuseAction::Reply(encode_fuse_reply(unique, -LINUX_EINVAL, &[]));
                }

                let Some((req_in, req_out)) = ioctl_retry_sizes(cmd) else {
                    return FuseAction::Reply(encode_fuse_reply(unique, -LINUX_ENOTTY, &[]));
                };

                if in_size < req_in || out_size < req_out {
                    if (flags & FUSE_IOCTL_UNRESTRICTED) == 0 {
                        return FuseAction::Reply(encode_fuse_reply(unique, -LINUX_EFAULT, &[]));
                    }
                    // Tell kernel CUSE to copy user memory in/out matching the exact struct size.
                    let in_iovs = u32::from(req_in > 0);
                    let out_iovs = u32::from(req_out > 0);
                    let mut payload = Vec::with_capacity(
                        FUSE_IOCTL_OUT_LEN + ((in_iovs + out_iovs) as usize) * 16,
                    );
                    payload.extend_from_slice(&0i32.to_ne_bytes()); // result
                    payload.extend_from_slice(&FUSE_IOCTL_RETRY.to_ne_bytes());
                    payload.extend_from_slice(&in_iovs.to_ne_bytes());
                    payload.extend_from_slice(&out_iovs.to_ne_bytes());
                    if req_in > 0 {
                        payload.extend_from_slice(&arg.to_ne_bytes());
                        payload.extend_from_slice(&(req_in as u64).to_ne_bytes());
                    }
                    if req_out > 0 {
                        payload.extend_from_slice(&arg.to_ne_bytes());
                        payload.extend_from_slice(&(req_out as u64).to_ne_bytes());
                    }
                    return FuseAction::Reply(encode_fuse_reply(unique, 0, &payload));
                }

                if cmd == TIOCMIWAIT {
                    return FuseAction::ParkPinWait {
                        unique,
                        tiocm_mask: arg as u32,
                    };
                }

                let ioctl_arg_bytes;
                let effective_in: &[u8] = if cmd == TCSBRK && req_in == 0 {
                    ioctl_arg_bytes = (arg as u32).to_ne_bytes();
                    &ioctl_arg_bytes
                } else {
                    &arg_in[..req_in]
                };

                let res = {
                    let mut guard = self.handler.lock().await;
                    let r = guard.handle_ioctl(cmd, effective_in);
                    if r.is_ok() && is_termios_set_ioctl(cmd) {
                        let _ = self.config_tx.send(guard.config());
                    }
                    r
                };

                match res {
                    Ok(mut out_bytes) => {
                        if out_bytes.len() > out_size {
                            out_bytes.truncate(out_size);
                        }
                        let mut payload = Vec::with_capacity(FUSE_IOCTL_OUT_LEN + out_bytes.len());
                        payload.extend_from_slice(&0i32.to_ne_bytes()); // result = 0
                        payload.extend_from_slice(&0u32.to_ne_bytes()); // flags
                        payload.extend_from_slice(&0u32.to_ne_bytes()); // in_iovs
                        payload.extend_from_slice(&0u32.to_ne_bytes()); // out_iovs
                        payload.extend_from_slice(&out_bytes);
                        FuseAction::Reply(encode_fuse_reply(unique, 0, &payload))
                    }
                    Err(HalError::UnsupportedIoctl(_)) => {
                        FuseAction::Reply(encode_fuse_reply(unique, -LINUX_ENOTTY, &[]))
                    }
                    Err(_) => FuseAction::Reply(encode_fuse_reply(unique, -LINUX_EINVAL, &[])),
                }
            }
            _ => FuseAction::Reply(encode_fuse_reply(unique, -LINUX_ENOSYS, &[])),
        }
    }
}

#[cfg(target_os = "linux")]
mod linux_cuse {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use tokio::io::unix::AsyncFd;
    use tokio::sync::{mpsc, watch};

    fn validate_device_name(device_name: &str) -> Result<String> {
        let trimmed = device_name.trim();
        if trimmed.is_empty()
            || trimmed.contains('/')
            || trimmed.contains('\0')
            || trimmed.chars().any(|c| c.is_whitespace())
        {
            return Err(HalError::CuseInit(format!(
                "invalid CUSE device name '{device_name}'"
            )));
        }
        Ok(trimmed.to_string())
    }

    fn write_all_cuse(fd: RawFd, data: &[u8]) -> std::io::Result<()> {
        let res = unsafe { libc::write(fd, data.as_ptr().cast::<libc::c_void>(), data.len()) };
        if res < 0 {
            let err = std::io::Error::last_os_error();
            // ENOENT means the kernel already aborted the FUSE request (e.g., interrupted caller).
            if err.raw_os_error() == Some(libc::ENOENT) {
                return Ok(());
            }
            Err(err)
        } else {
            Ok(())
        }
    }

    pub(super) async fn run_cuse_loop(
        async_fd: AsyncFd<OwnedFd>,
        mut session: CuseFuseSession,
        tx_notify: Arc<Notify>,
        mut poll_wake_rx: mpsc::Receiver<()>,
        mut shutdown_rx: oneshot::Receiver<()>,
    ) {
        let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let mut parked: HashMap<u64, oneshot::Sender<()>> = HashMap::new();
        let mut dev_buf = vec![0u8; CUSE_READ_BUFFER_LEN];

        'cuse_loop: loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                Some(reply) = reply_rx.recv() => {
                    if write_all_cuse(async_fd.get_ref().as_raw_fd(), &reply).is_err() {
                        break;
                    }
                }
                Some(()) = poll_wake_rx.recv() => {
                    let waiters: Vec<u64> = {
                        let mut guard = session.poll_waiters.lock().unwrap_or_else(|e| e.into_inner());
                        std::mem::take(&mut *guard)
                    };
                    for kh in waiters {
                        let _ = write_all_cuse(
                            async_fd.get_ref().as_raw_fd(),
                            &encode_fuse_notify_poll(kh),
                        );
                    }
                }
                guard_res = async_fd.readable() => {
                    let Ok(mut guard) = guard_res else { break };
                    loop {
                        match guard.try_io(|inner| {
                            let res = unsafe {
                                libc::read(
                                    inner.as_raw_fd(),
                                    dev_buf.as_mut_ptr().cast::<libc::c_void>(),
                                    dev_buf.len(),
                                )
                            };
                            if res < 0 {
                                Err(std::io::Error::last_os_error())
                            } else {
                                Ok(res as usize)
                            }
                        }) {
                            Ok(Ok(0)) => break 'cuse_loop,
                            Ok(Ok(n)) => {
                                match session.process_packet(&dev_buf[..n]).await {
                                    FuseAction::Reply(reply) => {
                                        if write_all_cuse(async_fd.get_ref().as_raw_fd(), &reply).is_err() {
                                            break 'cuse_loop;
                                        }
                                    }
                                    FuseAction::Shutdown(reply) => {
                                        let _ = write_all_cuse(async_fd.get_ref().as_raw_fd(), &reply);
                                        break 'cuse_loop;
                                    }
                                    FuseAction::Interrupt { target_unique } => {
                                        if let Some(cancel) = parked.remove(&target_unique) {
                                            let _ = cancel.send(());
                                        }
                                    }
                                    FuseAction::ParkRead { unique, size } => {
                                        parked.retain(|_, ch| !ch.is_closed());
                                        let (cancel_tx, mut cancel_rx) = oneshot::channel();
                                        parked.insert(unique, cancel_tx);
                                        let tx_queue = Arc::clone(&session.tx_queue);
                                        let handler = Arc::clone(&session.handler);
                                        let tx_notify_ref = Arc::clone(&tx_notify);
                                        let reply_tx_ref = reply_tx.clone();
                                        tokio::spawn(async move {
                                            loop {
                                                let notified = tx_notify_ref.notified();
                                                tokio::pin!(notified);
                                                notified.as_mut().enable();

                                                let popped = {
                                                    let mut q = tx_queue.lock().unwrap_or_else(|e| e.into_inner());
                                                    if q.is_empty() {
                                                        None
                                                    } else {
                                                        let cnt = q.len().min(size);
                                                        Some(q.drain(..cnt).collect::<Vec<u8>>())
                                                    }
                                                };
                                                if let Some(chunk) = popped {
                                                    {
                                                        let mut h = handler.lock().await;
                                                        h.counters.rx = h.counters.rx.saturating_add(chunk.len() as u32);
                                                    }
                                                    let _ = reply_tx_ref.send(encode_fuse_reply(unique, 0, &chunk));
                                                    break;
                                                }

                                                tokio::select! {
                                                    _ = &mut cancel_rx => {
                                                        let _ = reply_tx_ref.send(encode_fuse_reply(unique, -LINUX_EINTR, &[]));
                                                        break;
                                                    }
                                                    _ = notified => {}
                                                }
                                            }
                                        });
                                    }
                                    FuseAction::ParkPinWait { unique, tiocm_mask } => {
                                        parked.retain(|_, ch| !ch.is_closed());
                                        let (cancel_tx, mut cancel_rx) = oneshot::channel();
                                        parked.insert(unique, cancel_tx);
                                        let handler = Arc::clone(&session.handler);
                                        let reply_tx_ref = reply_tx.clone();
                                        tokio::spawn(async move {
                                            let (rx, starting) = {
                                                let guard = handler.lock().await;
                                                guard.prepare_pin_wait(tiocm_mask)
                                            };
                                            let payload = [0u8; FUSE_IOCTL_OUT_LEN];
                                            tokio::select! {
                                                _ = &mut cancel_rx => {
                                                    let _ = reply_tx_ref.send(encode_fuse_reply(unique, -LINUX_EINTR, &[]));
                                                }
                                                res = await_pin_change(rx, starting, tiocm_mask, None) => {
                                                    let reply = match res {
                                                        Ok(_) => encode_fuse_reply(unique, 0, &payload),
                                                        Err(_) => encode_fuse_reply(unique, -LINUX_EIO, &[]),
                                                    };
                                                    let _ = reply_tx_ref.send(reply);
                                                }
                                            }
                                        });
                                    }
                                }
                            }
                            Ok(Err(e)) if e.raw_os_error() == Some(libc::EINTR) => continue,
                            Ok(Err(_)) => break 'cuse_loop,
                            Err(_would_block) => break,
                        }
                    }
                }
            }
        }
    }

    pub(super) fn open_linux_device(
        device_name: &str,
        handler: Arc<Mutex<SerialIoctlHandler>>,
        config_tx: watch::Sender<SerialConfig>,
        inbound_tx: mpsc::Sender<Vec<u8>>,
        tx_queue: Arc<StdMutex<VecDeque<u8>>>,
        tx_notify: Arc<Notify>,
        poll_wake_rx: mpsc::Receiver<()>,
    ) -> Result<oneshot::Sender<()>> {
        let valid_name = validate_device_name(device_name)?;
        let c_dev = CString::new("/dev/cuse").map_err(|e| HalError::CuseInit(e.to_string()))?;
        let raw_fd = unsafe { libc::open(c_dev.as_ptr(), libc::O_RDWR) };
        if raw_fd < 0 {
            return Err(HalError::OpenCuse(std::io::Error::last_os_error()));
        }
        let owned_fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };

        // Synchronous CUSE_INIT handshake guarantees `/dev/<device_name>` is registered
        // (or init errors fail loudly) before CuseEndpoint::open() returns.
        let mut pfd = libc::pollfd {
            fd: owned_fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let poll_res = unsafe { libc::poll(&mut pfd, 1, 2_000) };
        if poll_res < 0 {
            return Err(HalError::CuseInit(
                std::io::Error::last_os_error().to_string(),
            ));
        }
        if poll_res == 0 {
            return Err(HalError::CuseInit(
                "timed out waiting for kernel CUSE_INIT on /dev/cuse".into(),
            ));
        }

        let mut init_buf = [0u8; 512];
        let n = unsafe {
            libc::read(
                owned_fd.as_raw_fd(),
                init_buf.as_mut_ptr().cast::<libc::c_void>(),
                init_buf.len(),
            )
        };
        if n <= 0 {
            return Err(HalError::CuseInit(format!(
                "failed reading CUSE_INIT: {}",
                std::io::Error::last_os_error()
            )));
        }

        let poll_waiters = Arc::new(StdMutex::new(Vec::new()));
        let mut session = CuseFuseSession::new(
            valid_name,
            handler,
            config_tx,
            inbound_tx,
            tx_queue,
            poll_waiters,
        );

        let rt = tokio::runtime::Handle::try_current()
            .map_err(|e| HalError::CuseInit(format!("Tokio runtime required for CUSE I/O: {e}")))?;

        let init_packet = &init_buf[..n as usize];
        let init_action = futures_block_on_init(&rt, &mut session, init_packet);
        match init_action {
            FuseAction::Reply(reply) => {
                if !session.initialized {
                    return Err(HalError::CuseInit(
                        "kernel CUSE_INIT handshake was rejected".into(),
                    ));
                }
                write_all_cuse(owned_fd.as_raw_fd(), &reply).map_err(|e| {
                    HalError::CuseInit(format!("failed writing CUSE_INIT reply: {e}"))
                })?;
            }
            _ => {
                return Err(HalError::CuseInit(
                    "unexpected non-CUSE_INIT initial request from /dev/cuse".into(),
                ));
            }
        }

        // Switch /dev/cuse fd to non-blocking before attaching Tokio AsyncFd.
        let fl = unsafe { libc::fcntl(owned_fd.as_raw_fd(), libc::F_GETFL) };
        if fl < 0
            || unsafe { libc::fcntl(owned_fd.as_raw_fd(), libc::F_SETFL, fl | libc::O_NONBLOCK) }
                < 0
        {
            return Err(HalError::OpenCuse(std::io::Error::last_os_error()));
        }

        // SAFETY: `owned_fd` is an owned descriptor kept alive inside the AsyncFd;
        // nothing closes or replaces it behind the registration. `AsyncFd::new` is
        // deprecated for lacking I/O safety.
        let async_fd = match unsafe { AsyncFd::register(owned_fd) } {
            Ok(fd) => fd,
            Err(err) => {
                let (_fd, cause) = err.into_parts();
                return Err(HalError::OpenCuse(cause));
            }
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        rt.spawn(run_cuse_loop(
            async_fd,
            session,
            tx_notify,
            poll_wake_rx,
            shutdown_rx,
        ));
        Ok(shutdown_tx)
    }

    fn futures_block_on_init(
        _rt: &tokio::runtime::Handle,
        session: &mut CuseFuseSession,
        packet: &[u8],
    ) -> FuseAction {
        // CUSE_INIT processing uses no async waits; poll a single step deterministically even
        // when `CuseEndpoint::open` is called from inside a Tokio worker thread.
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        const VTABLE: RawWakerVTable =
            RawWakerVTable::new(|p| RawWaker::new(p, &VTABLE), |_| {}, |_| {}, |_| {});
        let raw = RawWaker::new(std::ptr::null(), &VTABLE);
        let waker = unsafe { Waker::from_raw(raw) };
        let mut cx = Context::from_waker(&waker);
        let fut = session.process_packet(packet);
        let mut pinned = std::pin::pin!(fut);
        match pinned.as_mut().poll(&mut cx) {
            Poll::Ready(action) => action,
            Poll::Pending => FuseAction::Reply(Vec::new()),
        }
    }
}

enum CuseIo {
    #[cfg(target_os = "linux")]
    Linux {
        inbound_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
        tx_queue: Arc<StdMutex<VecDeque<u8>>>,
        tx_notify: Arc<Notify>,
        poll_wake_tx: tokio::sync::mpsc::Sender<()>,
        config_rx: tokio::sync::watch::Receiver<SerialConfig>,
        _shutdown_tx: oneshot::Sender<()>,
    },
    Mock {
        master_stream: tokio::io::DuplexStream,
        mock_client: Option<tokio::io::DuplexStream>,
    },
}

/// Tier-2 Linux CUSE Character Device Endpoint supporting full modem pin emulation
pub struct CuseEndpoint {
    id: EndpointId,
    name: String,
    device_name: String,
    handler: Arc<Mutex<SerialIoctlHandler>>,
    throttler: TokenBucketThrottler,
    io: CuseIo,
}

impl CuseEndpoint {
    /// Opens a real Linux `/dev/cuse` character device node (creating `/dev/<device_name>`).
    /// Fails with `HalError::OpenCuse` on non-Linux hosts or when `/dev/cuse` is inaccessible.
    pub fn open(
        id: EndpointId,
        name: impl Into<String>,
        device_name: impl Into<String>,
        config: SerialConfig,
        initial_pins: PinState,
    ) -> Result<Self> {
        let name_str = name.into();
        let dev_str = device_name.into();

        #[cfg(target_os = "linux")]
        {
            let throttler = TokenBucketThrottler::new(config);
            let handler = Arc::new(Mutex::new(SerialIoctlHandler::new(initial_pins, config)));
            let (config_tx, config_rx) = tokio::sync::watch::channel(config);
            let (inbound_tx, inbound_rx) = tokio::sync::mpsc::channel(1024);
            let tx_queue = Arc::new(StdMutex::new(VecDeque::with_capacity(8192)));
            let tx_notify = Arc::new(Notify::new());
            let (poll_wake_tx, poll_wake_rx) = tokio::sync::mpsc::channel(1);

            let shutdown_tx = linux_cuse::open_linux_device(
                &dev_str,
                Arc::clone(&handler),
                config_tx,
                inbound_tx,
                Arc::clone(&tx_queue),
                Arc::clone(&tx_notify),
                poll_wake_rx,
            )?;

            Ok(Self {
                id,
                name: name_str,
                device_name: dev_str,
                handler,
                throttler,
                io: CuseIo::Linux {
                    inbound_rx,
                    tx_queue,
                    tx_notify,
                    poll_wake_tx,
                    config_rx,
                    _shutdown_tx: shutdown_tx,
                },
            })
        }

        #[cfg(not(target_os = "linux"))]
        {
            let _ = (id, name_str, dev_str, config, initial_pins);
            Err(HalError::OpenCuse(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "/dev/cuse character devices are only supported on Linux",
            )))
        }
    }

    /// Opens an explicit in-memory mock CUSE endpoint for testing without `/dev/cuse`.
    pub fn open_mock(
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
            io: CuseIo::Mock {
                master_stream,
                mock_client: Some(mock_client),
            },
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
    pub async fn dispatch_ioctl(&mut self, cmd: u32, arg_in: &[u8]) -> Result<Vec<u8>> {
        let result = {
            let mut guard = self.handler.lock().await;
            guard.handle_ioctl(cmd, arg_in)
        };
        if result.is_ok() && is_termios_set_ioctl(cmd) {
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
        match &mut self.io {
            CuseIo::Mock { mock_client, .. } => mock_client.take(),
            #[cfg(target_os = "linux")]
            CuseIo::Linux { .. } => None,
        }
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
            let payload = match &mut self.io {
                #[cfg(target_os = "linux")]
                CuseIo::Linux { inbound_rx, .. } => match inbound_rx.recv().await {
                    Some(bytes) => Bytes::from(bytes),
                    None => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "CUSE session closed",
                        ));
                    }
                },
                CuseIo::Mock { master_stream, .. } => {
                    use tokio::io::AsyncReadExt;
                    let mut buf = [0u8; 4096];
                    let n = master_stream.read(&mut buf).await?;
                    if n == 0 {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "CUSE client disconnected",
                        ));
                    }
                    Bytes::copy_from_slice(&buf[..n])
                }
            };

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
                payload,
            })
        })
    }

    fn send(
        &mut self,
        event: SerialEvent,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send + '_>> {
        Box::pin(async move {
            if !event.payload.is_empty() {
                #[cfg(target_os = "linux")]
                if let CuseIo::Linux { config_rx, .. } = &mut self.io
                    && config_rx.has_changed().unwrap_or(false)
                {
                    let cfg = *config_rx.borrow_and_update();
                    self.throttler.update_config(cfg);
                }

                // Apply software token-bucket baud rate throttling
                self.throttler.throttle_async(event.payload.len()).await;

                match &mut self.io {
                    #[cfg(target_os = "linux")]
                    CuseIo::Linux {
                        tx_queue,
                        tx_notify,
                        poll_wake_tx,
                        ..
                    } => {
                        {
                            let mut q = tx_queue.lock().unwrap_or_else(|e| e.into_inner());
                            q.extend(event.payload.as_ref());
                        }
                        tx_notify.notify_waiters();
                        let _ = poll_wake_tx.try_send(());
                    }
                    CuseIo::Mock { master_stream, .. } => {
                        use tokio::io::AsyncWriteExt;
                        master_stream.write_all(&event.payload).await?;
                    }
                }
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
            guard.set_config(cfg);
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn make_fuse_in_frame(opcode: u32, unique: u64, body: &[u8]) -> Vec<u8> {
        let len = (FUSE_IN_HEADER_LEN + body.len()) as u32;
        let mut frame = Vec::with_capacity(len as usize);
        frame.extend_from_slice(&len.to_ne_bytes());
        frame.extend_from_slice(&opcode.to_ne_bytes());
        frame.extend_from_slice(&unique.to_ne_bytes());
        frame.extend_from_slice(&0u64.to_ne_bytes()); // nodeid
        frame.extend_from_slice(&0u32.to_ne_bytes()); // uid
        frame.extend_from_slice(&0u32.to_ne_bytes()); // gid
        frame.extend_from_slice(&123u32.to_ne_bytes()); // pid
        frame.extend_from_slice(&0u32.to_ne_bytes()); // padding
        frame.extend_from_slice(body);
        frame
    }

    fn parse_fuse_out(frame: &[u8]) -> (u32, i32, u64, &[u8]) {
        assert!(frame.len() >= FUSE_OUT_HEADER_LEN);
        let len = read_u32_ne(frame, 0);
        assert_eq!(len as usize, frame.len());
        let error = i32::from_ne_bytes([frame[4], frame[5], frame[6], frame[7]]);
        let unique = read_u64_ne(frame, 8);
        (len, error, unique, &frame[FUSE_OUT_HEADER_LEN..])
    }

    #[test]
    fn test_kernel_struct_sizes_match_linux_uapi() {
        let cfg = SerialConfig {
            baud_rate: 115_200,
            data_bits: 7,
            stop_bits: 2,
            parity: 2,
        };
        let termios = KernelTermios::from_config(&cfg);
        assert_eq!(termios.to_bytes().len(), 36);
        assert_eq!(termios.to_termios2_bytes(115_200).len(), 44);

        let decoded = KernelTermios::from_bytes(&termios.to_bytes())
            .unwrap()
            .decode_config(9600, None)
            .unwrap();
        assert_eq!(decoded.baud_rate, 115_200);
        assert_eq!(decoded.data_bits, 7);
        assert_eq!(decoded.stop_bits, 2);
        assert_eq!(decoded.parity, 2);

        // Non-standard baud rate uses BOTHER + c_ospeed in termios2.
        let custom_cfg = SerialConfig {
            baud_rate: 250_000,
            ..Default::default()
        };
        let t2_bytes = KernelTermios::from_config(&custom_cfg).to_termios2_bytes(250_000);
        let (parsed_t2, ispeed, ospeed) = KernelTermios::from_termios2_bytes(&t2_bytes).unwrap();
        assert_eq!(ospeed, 250_000);
        let custom_decoded = parsed_t2
            .decode_config(9600, Some((ispeed, ospeed)))
            .unwrap();
        assert_eq!(custom_decoded.baud_rate, 250_000);

        let counters = SerialCounters {
            cts: 1,
            dsr: 2,
            rng: 3,
            dcd: 4,
            rx: 50,
            tx: 60,
            frame: 5,
            overrun: 6,
            parity: 7,
            brk: 8,
            buf_overrun: 9,
        };
        let raw_counters = counters.to_bytes();
        assert_eq!(raw_counters.len(), 80);
        assert_eq!(SerialCounters::from_bytes(&raw_counters).unwrap(), counters);
    }

    #[tokio::test]
    async fn test_cuse_ioctl_modem_get_set() {
        let mut handler = SerialIoctlHandler::new(PinState::empty(), SerialConfig::default());

        let res = handler.handle_ioctl(TIOCMGET, &[]).unwrap();
        let bits = u32::from_ne_bytes([res[0], res[1], res[2], res[3]]);
        assert_eq!(bits, 0);

        let set_bits = (TIOCM_DTR | TIOCM_RTS).to_ne_bytes();
        handler.handle_ioctl(TIOCMSET, &set_bits).unwrap();

        assert!(handler.pins().contains(PinState::DTR));
        assert!(handler.pins().contains(PinState::RTS));
        assert!(!handler.pins().contains(PinState::CTS));

        let res2 = handler.handle_ioctl(TIOCMGET, &[]).unwrap();
        let bits2 = u32::from_ne_bytes([res2[0], res2[1], res2[2], res2[3]]);
        assert_eq!(bits2, TIOCM_DTR | TIOCM_RTS);

        let bis_bits = TIOCM_CTS.to_ne_bytes();
        handler.handle_ioctl(TIOCMBIS, &bis_bits).unwrap();
        assert!(handler.pins().contains(PinState::CTS));

        let bic_bits = TIOCM_RTS.to_ne_bytes();
        handler.handle_ioctl(TIOCMBIC, &bic_bits).unwrap();
        assert!(!handler.pins().contains(PinState::RTS));
        assert!(handler.pins().contains(PinState::DTR));
        assert!(handler.pins().contains(PinState::CTS));
    }

    #[tokio::test]
    async fn test_cuse_break_and_counters() {
        let mut handler = SerialIoctlHandler::new(PinState::empty(), SerialConfig::default());

        handler.handle_ioctl(TIOCSBRK, &[]).unwrap();
        assert!(handler.pins().contains(PinState::BREAK));
        assert_eq!(handler.counters().brk, 1);

        handler.handle_ioctl(TIOCCBRK, &[]).unwrap();
        assert!(!handler.pins().contains(PinState::BREAK));
        assert_eq!(handler.counters().brk, 1);

        handler.handle_ioctl(TCSBRK, &[]).unwrap();
        assert_eq!(handler.counters().brk, 2);

        let icount = handler.handle_ioctl(TIOCGICOUNT, &[]).unwrap();
        assert_eq!(icount.len(), SERIAL_ICOUNTER_SIZE);
        assert_eq!(SerialCounters::from_bytes(&icount).unwrap().brk, 2);
    }

    #[tokio::test]
    async fn test_cuse_tcsbrk_nonzero_arg_does_not_emit_break() {
        let mut handler = SerialIoctlHandler::new(PinState::empty(), SerialConfig::default());

        handler
            .handle_ioctl(TCSBRK, &1u32.to_ne_bytes())
            .expect("TCSBRK drain");
        assert_eq!(handler.counters().brk, 0);
        assert!(!handler.pins().contains(PinState::BREAK));

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
        let mut ep =
            CuseEndpoint::open_mock(9, "cuse_baud", "ttyOmniBaud", config, PinState::empty())
                .expect("open mock cuse endpoint");
        assert_eq!(ep.throttler().byte_rate(), 11_520.0);

        // Short/bogus struct sizes must be rejected instead of silently accepted.
        assert!(matches!(
            ep.dispatch_ioctl(TCSETS, &[0u8; 8]).await,
            Err(HalError::InvalidIoctlArg(_))
        ));

        let termios_9600 = KernelTermios::from_config(&SerialConfig {
            baud_rate: 9_600,
            ..config
        });
        ep.dispatch_ioctl(TCSETS, &termios_9600.to_bytes())
            .await
            .expect("TCSETS");

        assert_eq!(ep.handler().lock().await.config().baud_rate, 9_600);
        assert_eq!(ep.throttler().config().baud_rate, 9_600);
        assert_eq!(ep.throttler().byte_rate(), 960.0);
    }

    #[tokio::test]
    async fn test_cuse_tiocmiwait_wakeup() {
        let ep = CuseEndpoint::open_mock(
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

        tokio::time::sleep(Duration::from_millis(20)).await;

        {
            let mut guard = ep_arc.handler().lock().await;
            guard.set_pins(PinState::CTS);
        }

        let result = wait_handle.await.unwrap().expect("wait success");
        assert!(result.contains(PinState::CTS));
        assert_eq!(ep_arc.handler().lock().await.counters().cts, 1);
    }

    #[tokio::test]
    async fn test_cuse_fuse_protocol_handshake_retry_ioctl_and_io() {
        let handler = Arc::new(Mutex::new(SerialIoctlHandler::new(
            PinState::DTR | PinState::RTS,
            SerialConfig {
                baud_rate: 115_200,
                data_bits: 8,
                stop_bits: 1,
                parity: 0,
            },
        )));
        let (config_tx, mut config_rx) = tokio::sync::watch::channel(SerialConfig::default());
        let (inbound_tx, mut inbound_rx) = tokio::sync::mpsc::channel(16);
        let tx_queue = Arc::new(StdMutex::new(VecDeque::new()));
        let poll_waiters = Arc::new(StdMutex::new(Vec::new()));

        let mut session = CuseFuseSession::new(
            "ttyOmni0".into(),
            Arc::clone(&handler),
            config_tx,
            inbound_tx,
            Arc::clone(&tx_queue),
            poll_waiters,
        );

        // 1. CUSE_INIT handshake emits cuse_init_out (72B) + "DEVNAME=ttyOmni0\0" + UNRESTRICTED_IOCTL.
        let mut init_in = [0u8; CUSE_INIT_IN_LEN];
        init_in[0..4].copy_from_slice(&FUSE_KERNEL_VERSION.to_ne_bytes());
        init_in[4..8].copy_from_slice(&35u32.to_ne_bytes());
        init_in[12..16].copy_from_slice(&CUSE_UNRESTRICTED_IOCTL.to_ne_bytes());
        let FuseAction::Reply(init_reply) = session
            .process_packet(&make_fuse_in_frame(CUSE_INIT, 1, &init_in))
            .await
        else {
            panic!("expected synchronous CUSE_INIT reply");
        };
        let (_, err, uniq, body) = parse_fuse_out(&init_reply);
        assert_eq!((err, uniq), (0, 1));
        assert_eq!(read_u32_ne(body, 0), FUSE_KERNEL_VERSION);
        assert_eq!(read_u32_ne(body, 12), CUSE_UNRESTRICTED_IOCTL);
        assert_eq!(&body[CUSE_INIT_OUT_LEN..], b"DEVNAME=ttyOmni0\0");

        // 2. FUSE_OPEN returns DIRECT_IO | NONSEEKABLE.
        let FuseAction::Reply(open_reply) = session
            .process_packet(&make_fuse_in_frame(FUSE_OPEN, 2, &[0u8; 8]))
            .await
        else {
            panic!("expected FUSE_OPEN reply");
        };
        let (_, err, _, body) = parse_fuse_out(&open_reply);
        assert_eq!(err, 0);
        assert_eq!(read_u32_ne(body, 8), FOPEN_DIRECT_IO | FOPEN_NONSEEKABLE);

        // 3. Legacy TCGETS arrives unrestricted with out_size=0 -> must respond FUSE_IOCTL_RETRY (36B).
        let mut ioctl_req = [0u8; FUSE_IOCTL_IN_LEN];
        ioctl_req[8..12].copy_from_slice(&FUSE_IOCTL_UNRESTRICTED.to_ne_bytes());
        ioctl_req[12..16].copy_from_slice(&TCGETS.to_ne_bytes());
        ioctl_req[16..24].copy_from_slice(&0x7FFF_1234u64.to_ne_bytes());
        let FuseAction::Reply(retry_reply) = session
            .process_packet(&make_fuse_in_frame(FUSE_IOCTL, 3, &ioctl_req))
            .await
        else {
            panic!("expected FUSE_IOCTL_RETRY reply");
        };
        let (_, err, _, body) = parse_fuse_out(&retry_reply);
        assert_eq!(err, 0);
        assert_eq!(read_u32_ne(body, 4), FUSE_IOCTL_RETRY);
        assert_eq!(read_u32_ne(body, 12), 1); // out_iovs == 1
        assert_eq!(read_u64_ne(body, 16), 0x7FFF_1234u64);
        assert_eq!(read_u64_ne(body, 24), KERNEL_TERMIOS_SIZE as u64);

        // 4. Retried TCGETS returns 36-byte Linux struct termios; TCSETS updates config + watch channel.
        ioctl_req[28..32].copy_from_slice(&(KERNEL_TERMIOS_SIZE as u32).to_ne_bytes());
        let FuseAction::Reply(tcgets_reply) = session
            .process_packet(&make_fuse_in_frame(FUSE_IOCTL, 4, &ioctl_req))
            .await
        else {
            panic!("expected TCGETS reply");
        };
        let (_, err, _, body) = parse_fuse_out(&tcgets_reply);
        assert_eq!(err, 0);
        assert_eq!(body.len(), FUSE_IOCTL_OUT_LEN + KERNEL_TERMIOS_SIZE);

        let termios_9600 = KernelTermios::from_config(&SerialConfig {
            baud_rate: 9600,
            data_bits: 7,
            stop_bits: 1,
            parity: 1,
        });
        let mut tcsets_frame = Vec::with_capacity(FUSE_IOCTL_IN_LEN + KERNEL_TERMIOS_SIZE);
        let mut tcsets_in = [0u8; FUSE_IOCTL_IN_LEN];
        tcsets_in[8..12].copy_from_slice(&FUSE_IOCTL_UNRESTRICTED.to_ne_bytes());
        tcsets_in[12..16].copy_from_slice(&TCSETS.to_ne_bytes());
        tcsets_in[24..28].copy_from_slice(&(KERNEL_TERMIOS_SIZE as u32).to_ne_bytes());
        tcsets_frame.extend_from_slice(&tcsets_in);
        tcsets_frame.extend_from_slice(&termios_9600.to_bytes());
        let FuseAction::Reply(tcsets_reply) = session
            .process_packet(&make_fuse_in_frame(FUSE_IOCTL, 5, &tcsets_frame))
            .await
        else {
            panic!("expected TCSETS reply");
        };
        assert_eq!(parse_fuse_out(&tcsets_reply).1, 0);
        assert!(config_rx.has_changed().unwrap());
        assert_eq!(config_rx.borrow_and_update().baud_rate, 9600);

        // 5. FUSE_WRITE pushes bytes into inbound_rx; non-blocking FUSE_READ drains tx_queue or returns -EAGAIN.
        let mut write_frame = Vec::with_capacity(FUSE_WRITE_IN_LEN + 4);
        let mut write_in = [0u8; FUSE_WRITE_IN_LEN];
        write_in[16..20].copy_from_slice(&4u32.to_ne_bytes());
        write_frame.extend_from_slice(&write_in);
        write_frame.extend_from_slice(b"PING");
        let FuseAction::Reply(write_reply) = session
            .process_packet(&make_fuse_in_frame(FUSE_WRITE, 6, &write_frame))
            .await
        else {
            panic!("expected FUSE_WRITE reply");
        };
        assert_eq!(parse_fuse_out(&write_reply).1, 0);
        assert_eq!(inbound_rx.recv().await.unwrap(), b"PING");

        tx_queue.lock().unwrap().extend(b"PONG");
        let mut read_in = [0u8; FUSE_READ_IN_LEN];
        read_in[16..20].copy_from_slice(&16u32.to_ne_bytes());
        read_in[32..36].copy_from_slice(&LINUX_O_NONBLOCK.to_ne_bytes());
        let FuseAction::Reply(read_reply) = session
            .process_packet(&make_fuse_in_frame(FUSE_READ, 7, &read_in))
            .await
        else {
            panic!("expected FUSE_READ reply");
        };
        let (_, err, _, body) = parse_fuse_out(&read_reply);
        assert_eq!(err, 0);
        assert_eq!(body, b"PONG");

        // Second non-blocking read with empty tx_queue yields -EAGAIN.
        let FuseAction::Reply(eagain_reply) = session
            .process_packet(&make_fuse_in_frame(FUSE_READ, 8, &read_in))
            .await
        else {
            panic!("expected non-blocking EAGAIN reply");
        };
        assert_eq!(parse_fuse_out(&eagain_reply).1, -LINUX_EAGAIN);
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn test_cuse_open_fails_loudly_when_not_on_linux() {
        let err = match CuseEndpoint::open(
            1,
            "cuse_real",
            "ttyOmniReal",
            SerialConfig::default(),
            PinState::empty(),
        ) {
            Ok(_) => panic!("CuseEndpoint::open must not succeed silently without /dev/cuse"),
            Err(e) => e,
        };
        assert!(matches!(err, HalError::OpenCuse(_)));
    }

    #[tokio::test]
    async fn test_cuse_endpoint_io_and_throttling() {
        let config = SerialConfig {
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
        };

        let mut ep = CuseEndpoint::open_mock(
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

        client
            .write_all(b"HELLO_CUSE\n")
            .await
            .expect("client write");

        let event = ep.recv().await.expect("ep recv");
        assert_eq!(event.source_id, 5);
        assert_eq!(&event.payload[..], b"HELLO_CUSE\n");
        assert_eq!(event.pins, PinState::DTR | PinState::RTS);

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
