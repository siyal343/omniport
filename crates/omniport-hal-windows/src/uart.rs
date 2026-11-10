use crate::error::{HalError, Result};
use bytes::Bytes;
use omniport_core::{
    AsyncEndpoint, EndpointId, FrameFlags, PinState, SerialConfig, SerialEvent,
    TokenBucketThrottler,
};
use std::future::Future;
use std::pin::Pin;

#[cfg(windows)]
use windows_sys::Win32::Devices::Communication::*;
#[cfg(windows)]
use windows_sys::Win32::Foundation::*;
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::*;
#[cfg(windows)]
use windows_sys::Win32::System::Threading::*;

/// Helper to ensure COM port name uses the Win32 device prefix `\\.\`
pub fn format_com_path(name: &str) -> String {
    if name.starts_with(r"\\.\") {
        name.to_string()
    } else {
        format!(r"\\.\{}", name)
    }
}

pub fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Rejects serial configurations that cannot be programmed into a UART.
/// Invalid values previously reached the DCB layer (or the token bucket,
/// where baud_rate == 0 causes a division by zero and an effectively infinite
/// throttle delay) instead of failing at the API boundary.
pub fn validate_config(cfg: &SerialConfig) -> Result<()> {
    if cfg.baud_rate == 0 {
        return Err(HalError::InvalidParameter(
            "baud_rate must be greater than 0".to_string(),
        ));
    }
    if !(5..=8).contains(&cfg.data_bits) {
        return Err(HalError::InvalidParameter(format!(
            "data_bits must be 5..=8, got {}",
            cfg.data_bits
        )));
    }
    if !(1..=3).contains(&cfg.stop_bits) {
        return Err(HalError::InvalidParameter(format!(
            "stop_bits must be 1, 2 or 3 (1.5), got {}",
            cfg.stop_bits
        )));
    }
    if cfg.parity > 2 {
        return Err(HalError::InvalidParameter(format!(
            "parity must be 0 (none), 1 (odd) or 2 (even), got {}",
            cfg.parity
        )));
    }
    Ok(())
}

#[cfg(windows)]
struct SafeHandle(HANDLE);

#[cfg(windows)]
unsafe impl Send for SafeHandle {}
#[cfg(windows)]
unsafe impl Sync for SafeHandle {}

#[cfg(windows)]
impl Drop for SafeHandle {
    fn drop(&mut self) {
        if self.0 != INVALID_HANDLE_VALUE && !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

enum SerialIo {
    #[cfg(windows)]
    Hardware { handle: std::sync::Arc<SafeHandle> },
    Mock {
        master: tokio::io::DuplexStream,
        peer: Option<tokio::io::DuplexStream>,
    },
}

/// Windows Overlapped Win32 UART Endpoint
pub struct Win32SerialEndpoint {
    id: EndpointId,
    name: String,
    port_name: String,
    config: SerialConfig,
    pins: PinState,
    throttler: TokenBucketThrottler,
    io: SerialIo,
}

impl Win32SerialEndpoint {
    /// Opens a virtual or physical COM port using Win32 Overlapped I/O
    #[cfg(windows)]
    pub fn open(
        id: EndpointId,
        name: impl Into<String>,
        port_name: impl Into<String>,
        config: SerialConfig,
    ) -> Result<Self> {
        let port_str = port_name.into();
        validate_config(&config)?;
        let full_path = format_com_path(&port_str);
        let wide_path = to_wide_null(&full_path);

        let handle = unsafe {
            CreateFileW(
                wide_path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                std::ptr::null_mut(),
            )
        };

        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return Err(HalError::OpenPort {
                port: port_str,
                source: std::io::Error::last_os_error(),
            });
        }

        let safe_handle = std::sync::Arc::new(SafeHandle(handle));

        // Configure DCB baud rate, data bits, parity, stop bits
        apply_dcb_config(handle, &config)?;

        // Configure non-blocking timeouts
        apply_comm_timeouts(handle)?;

        let throttler = TokenBucketThrottler::new(config);

        Ok(Self {
            id,
            name: name.into(),
            port_name: port_str,
            config,
            pins: PinState::empty(),
            throttler,
            io: SerialIo::Hardware {
                handle: safe_handle,
            },
        })
    }

    /// Opens an in-memory mock UART endpoint for testing on any OS without hardware
    pub fn open_mock(
        id: EndpointId,
        name: impl Into<String>,
        port_name: impl Into<String>,
        config: SerialConfig,
    ) -> Result<Self> {
        validate_config(&config)?;
        let (master, peer) = tokio::io::duplex(65536);
        let throttler = TokenBucketThrottler::new(config);

        Ok(Self {
            id,
            name: name.into(),
            port_name: port_name.into(),
            config,
            pins: PinState::empty(),
            throttler,
            io: SerialIo::Mock {
                master,
                peer: Some(peer),
            },
        })
    }

    pub fn port_name(&self) -> &str {
        &self.port_name
    }

    pub fn current_config(&self) -> &SerialConfig {
        &self.config
    }

    pub fn throttler(&self) -> &TokenBucketThrottler {
        &self.throttler
    }

    pub fn throttler_mut(&mut self) -> &mut TokenBucketThrottler {
        &mut self.throttler
    }

    /// Queries hardware modem status lines (CTS, DSR, RI, DCD)
    pub fn query_modem_status(&mut self) -> Result<PinState> {
        match &self.io {
            #[cfg(windows)]
            SerialIo::Hardware { handle } => {
                let mut status = 0u32;
                let res = unsafe { GetCommModemStatus(handle.0, &mut status) };
                if res == 0 {
                    return Err(HalError::ModemStatus(std::io::Error::last_os_error()));
                }
                let mut modem_pins = PinState::empty();
                if (status & MS_CTS_ON) != 0 {
                    modem_pins |= PinState::CTS;
                }
                if (status & MS_DSR_ON) != 0 {
                    modem_pins |= PinState::DSR;
                }
                if (status & MS_RING_ON) != 0 {
                    modem_pins |= PinState::RI;
                }
                if (status & MS_RLSD_ON) != 0 {
                    modem_pins |= PinState::DCD;
                }

                // Preserve local DTR / RTS / BREAK states
                self.pins =
                    (self.pins & (PinState::DTR | PinState::RTS | PinState::BREAK)) | modem_pins;
                Ok(self.pins)
            }
            SerialIo::Mock { .. } => Ok(self.pins),
        }
    }

    /// Extracts the mock peer duplex stream for unit and integration testing
    pub fn take_mock_peer(&mut self) -> Option<tokio::io::DuplexStream> {
        match &mut self.io {
            SerialIo::Mock { peer, .. } => peer.take(),
            #[cfg(windows)]
            _ => None,
        }
    }
}

impl AsyncEndpoint for Win32SerialEndpoint {
    fn id(&self) -> EndpointId {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn recv(&mut self) -> Pin<Box<dyn Future<Output = std::io::Result<SerialEvent>> + Send + '_>> {
        Box::pin(async move {
            let mut buf = [0u8; 4096];
            let n = match &mut self.io {
                #[cfg(windows)]
                SerialIo::Hardware { handle } => {
                    let h = std::sync::Arc::clone(handle);
                    let (lbuf, cnt) = tokio::task::spawn_blocking(move || {
                        let mut bytes_read = 0u32;
                        let mut overlapped: windows_sys::Win32::System::IO::OVERLAPPED =
                            unsafe { std::mem::zeroed() };
                        let event = unsafe {
                            CreateEventW(std::ptr::null(), TRUE, FALSE, std::ptr::null())
                        };
                        if event.is_null() {
                            return Err(std::io::Error::last_os_error());
                        }
                        overlapped.hEvent = event;

                        let mut lbuf = [0u8; 4096];
                        let res = unsafe {
                            ReadFile(
                                h.0,
                                lbuf.as_mut_ptr() as _,
                                lbuf.len() as u32,
                                &mut bytes_read,
                                &mut overlapped,
                            )
                        };

                        if res == 0 {
                            let err = unsafe { GetLastError() };
                            if err != ERROR_IO_PENDING {
                                // The read failed outright; report it instead of
                                // emitting a silent zero-length event.
                                unsafe { CloseHandle(event) };
                                return Err(std::io::Error::from_raw_os_error(err as i32));
                            }
                            unsafe { WaitForSingleObject(event, INFINITE) };
                            if unsafe {
                                windows_sys::Win32::System::IO::GetOverlappedResult(
                                    h.0,
                                    &overlapped,
                                    &mut bytes_read,
                                    TRUE,
                                )
                            } == 0
                            {
                                let io_err = std::io::Error::last_os_error();
                                unsafe { CloseHandle(event) };
                                return Err(io_err);
                            }
                        }
                        unsafe { CloseHandle(event) };
                        Ok((lbuf, bytes_read as usize))
                    })
                    .await
                    .map_err(std::io::Error::other)??;
                    buf[..cnt].copy_from_slice(&lbuf[..cnt]);
                    cnt
                }
                SerialIo::Mock { master, .. } => {
                    use tokio::io::AsyncReadExt;
                    let cnt = master.read(&mut buf).await?;
                    if cnt == 0 {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "COM port disconnected",
                        ));
                    }
                    cnt
                }
            };

            let now_ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;

            Ok(SerialEvent {
                timestamp_ns: now_ns,
                source_id: self.id,
                flags: FrameFlags::empty(),
                pins: self.pins,
                payload: Bytes::copy_from_slice(&buf[..n]),
            })
        })
    }

    fn send(
        &mut self,
        event: SerialEvent,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send + '_>> {
        Box::pin(async move {
            if !event.payload.is_empty() {
                // Apply software token-bucket baud rate throttling
                self.throttler.throttle_async(event.payload.len()).await;

                match &mut self.io {
                    #[cfg(windows)]
                    SerialIo::Hardware { handle } => {
                        let h = std::sync::Arc::clone(handle);
                        let payload = event.payload.clone();
                        tokio::task::spawn_blocking(move || {
                            let mut bytes_written = 0u32;
                            let mut overlapped: windows_sys::Win32::System::IO::OVERLAPPED =
                                unsafe { std::mem::zeroed() };
                            let event = unsafe {
                                CreateEventW(std::ptr::null(), TRUE, FALSE, std::ptr::null())
                            };
                            if event.is_null() {
                                return Err(std::io::Error::last_os_error());
                            }
                            overlapped.hEvent = event;

                            let res = unsafe {
                                WriteFile(
                                    h.0,
                                    payload.as_ptr() as _,
                                    payload.len() as u32,
                                    &mut bytes_written,
                                    &mut overlapped,
                                )
                            };

                            if res == 0 {
                                let err = unsafe { GetLastError() };
                                if err != ERROR_IO_PENDING {
                                    unsafe { CloseHandle(event) };
                                    return Err(std::io::Error::from_raw_os_error(err as i32));
                                }
                                unsafe { WaitForSingleObject(event, INFINITE) };
                                if unsafe {
                                    windows_sys::Win32::System::IO::GetOverlappedResult(
                                        h.0,
                                        &overlapped,
                                        &mut bytes_written,
                                        TRUE,
                                    )
                                } == 0
                                {
                                    let io_err = std::io::Error::last_os_error();
                                    unsafe { CloseHandle(event) };
                                    return Err(io_err);
                                }
                            }
                            unsafe { CloseHandle(event) };
                            Ok::<(), std::io::Error>(())
                        })
                        .await
                        .map_err(std::io::Error::other)??;
                    }
                    SerialIo::Mock { master, .. } => {
                        use tokio::io::AsyncWriteExt;
                        master.write_all(&event.payload).await?;
                    }
                }
            }
            Ok(())
        })
    }

    fn set_pins(&mut self, pins: PinState) -> std::io::Result<()> {
        self.pins = pins;
        match &self.io {
            #[cfg(windows)]
            SerialIo::Hardware { handle } => {
                let h = handle.0;
                unsafe {
                    if pins.contains(PinState::DTR) {
                        EscapeCommFunction(h, SETDTR);
                    } else {
                        EscapeCommFunction(h, CLRDTR);
                    }

                    if pins.contains(PinState::RTS) {
                        EscapeCommFunction(h, SETRTS);
                    } else {
                        EscapeCommFunction(h, CLRRTS);
                    }

                    if pins.contains(PinState::BREAK) {
                        EscapeCommFunction(h, SETBREAK);
                    } else {
                        EscapeCommFunction(h, CLRBREAK);
                    }
                }
                Ok(())
            }
            SerialIo::Mock { .. } => Ok(()),
        }
    }

    fn configure(&mut self, cfg: SerialConfig) -> std::io::Result<()> {
        validate_config(&cfg).map_err(std::io::Error::other)?;
        self.config = cfg;
        self.throttler.update_config(cfg);

        match &self.io {
            #[cfg(windows)]
            SerialIo::Hardware { handle } => {
                apply_dcb_config(handle.0, &cfg).map_err(std::io::Error::other)?;
                Ok(())
            }
            SerialIo::Mock { .. } => Ok(()),
        }
    }
}

#[cfg(windows)]
fn apply_dcb_config(handle: HANDLE, cfg: &SerialConfig) -> Result<()> {
    validate_config(cfg)?;

    let mut dcb: DCB = unsafe { std::mem::zeroed() };
    dcb.DCBlength = std::mem::size_of::<DCB>() as u32;

    if unsafe { GetCommState(handle, &mut dcb) } == 0 {
        return Err(HalError::CommState(std::io::Error::last_os_error()));
    }

    configure_dcb(&mut dcb, cfg)?;

    if unsafe { SetCommState(handle, &dcb) } == 0 {
        return Err(HalError::CommState(std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Applies the requested line settings to a DCB obtained from `GetCommState`
/// without clobbering the control flags Windows filled in.
///
/// The `fBinary` flag lives in the same `_bitfield` word as `fDtrControl`
/// (bits 4-5) and `fRtsControl` (bits 12-13); assigning the whole bitfield used
/// to disable software DTR/RTS control, so `EscapeCommFunction` became a no-op.
#[cfg(windows)]
fn configure_dcb(dcb: &mut DCB, cfg: &SerialConfig) -> Result<()> {
    validate_config(cfg)?;

    dcb.BaudRate = cfg.baud_rate;
    dcb.ByteSize = cfg.data_bits;

    dcb.StopBits = match cfg.stop_bits {
        2 => TWOSTOPBITS,
        3 => ONE5STOPBITS,
        _ => ONESTOPBIT,
    };

    dcb.Parity = match cfg.parity {
        1 => ODDPARITY,
        2 => EVENPARITY,
        _ => NOPARITY,
    };
    // fParity enables parity-error checking and must follow the parity mode.
    if cfg.parity != 0 {
        dcb._bitfield |= 0x0000_0002; // fParity
    } else {
        dcb._bitfield &= !0x0000_0002;
    }

    // Set binary mode while preserving every other control bit.
    dcb._bitfield |= 0x0000_0001; // fBinary = 1
    Ok(())
}

#[cfg(windows)]
fn apply_comm_timeouts(handle: HANDLE) -> Result<()> {
    let timeouts = COMMTIMEOUTS {
        ReadIntervalTimeout: u32::MAX, // Immediate non-blocking return
        ReadTotalTimeoutMultiplier: 0,
        ReadTotalTimeoutConstant: 0,
        WriteTotalTimeoutMultiplier: 0,
        WriteTotalTimeoutConstant: 500, // 500 ms write timeout
    };

    if unsafe { SetCommTimeouts(handle, &timeouts) } == 0 {
        return Err(HalError::CommTimeouts(std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn test_mock_uart_endpoint_send_recv() {
        let config = SerialConfig {
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
        };

        let mut uart =
            Win32SerialEndpoint::open_mock(1, "uart1", "COM10", config).expect("open mock uart");

        assert_eq!(uart.id(), 1);
        assert_eq!(uart.name(), "uart1");
        assert_eq!(uart.port_name(), "COM10");

        let mut peer = uart.take_mock_peer().expect("take mock peer");

        // Test 1: Send from UART endpoint to peer
        let send_evt = SerialEvent {
            timestamp_ns: 1000,
            source_id: 1,
            flags: FrameFlags::empty(),
            pins: PinState::DTR,
            payload: Bytes::from_static(b"PING\r\n"),
        };
        uart.send(send_evt).await.expect("uart send");

        let mut peer_buf = vec![0u8; 6];
        peer.read_exact(&mut peer_buf).await.expect("peer read");
        assert_eq!(&peer_buf[..], b"PING\r\n");

        // Test 2: Send from peer to UART endpoint (recv)
        peer.write_all(b"PONG\r\n").await.expect("peer write");

        let recv_evt = uart.recv().await.expect("uart recv");
        assert_eq!(recv_evt.source_id, 1);
        assert_eq!(&recv_evt.payload[..], b"PONG\r\n");
    }

    #[tokio::test]
    async fn test_mock_uart_rejects_invalid_serial_configuration() {
        // baud_rate == 0 used to divide by zero in the token bucket and make a
        // send() wait for ~584 years instead of failing fast.
        let invalid = [
            SerialConfig {
                baud_rate: 0,
                ..SerialConfig::default()
            },
            SerialConfig {
                data_bits: 9,
                ..SerialConfig::default()
            },
            SerialConfig {
                data_bits: 4,
                ..SerialConfig::default()
            },
            SerialConfig {
                stop_bits: 0,
                ..SerialConfig::default()
            },
            SerialConfig {
                stop_bits: 4,
                ..SerialConfig::default()
            },
            SerialConfig {
                parity: 3,
                ..SerialConfig::default()
            },
        ];
        for (i, cfg) in invalid.iter().enumerate() {
            match Win32SerialEndpoint::open_mock(10 + i as u16, "bad", "COM99", *cfg) {
                Err(HalError::InvalidParameter(_)) => {}
                Err(other) => {
                    panic!("config {cfg:?} must be rejected with InvalidParameter, got {other:?}")
                }
                Ok(_) => panic!("config {cfg:?} must be rejected, but was accepted"),
            }
        }

        // configure() must enforce the same validation.
        let mut uart = Win32SerialEndpoint::open_mock(30, "ok", "COM30", SerialConfig::default())
            .expect("open mock uart");
        let err = uart
            .configure(SerialConfig {
                baud_rate: 0,
                ..SerialConfig::default()
            })
            .expect_err("zero baud rate must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::Other);
    }

    #[cfg(windows)]
    #[test]
    fn test_configure_dcb_preserves_control_flags() {
        // Bits as Windows returns them from GetCommState.
        const F_BINARY: u32 = 0x0000_0001;
        const F_PARITY: u32 = 0x0000_0002;
        const F_OUTX_CTS_FLOW: u32 = 0x0000_0004;
        const DTR_CONTROL_ENABLE: u32 = 0x0000_0010; // fDtrControl bits 4-5 = 01
        const RTS_CONTROL_ENABLE: u32 = 0x0000_1000; // fRtsControl bits 12-13 = 01

        let mut dcb: DCB = unsafe { std::mem::zeroed() };
        dcb.DCBlength = std::mem::size_of::<DCB>() as u32;
        dcb._bitfield = F_BINARY | F_OUTX_CTS_FLOW | DTR_CONTROL_ENABLE | RTS_CONTROL_ENABLE;

        let cfg = SerialConfig {
            baud_rate: 57_600,
            data_bits: 7,
            stop_bits: 2,
            parity: 2,
        };
        configure_dcb(&mut dcb, &cfg).expect("configure dcb");

        assert_eq!(dcb.BaudRate, 57_600);
        assert_eq!(dcb.ByteSize, 7);
        assert_eq!(dcb.StopBits, TWOSTOPBITS);
        assert_eq!(dcb.Parity, EVENPARITY);
        assert_eq!(dcb._bitfield & F_BINARY, F_BINARY, "fBinary must be set");
        assert_eq!(
            dcb._bitfield & DTR_CONTROL_ENABLE,
            DTR_CONTROL_ENABLE,
            "fDtrControl must stay enabled for EscapeCommFunction(DTR)"
        );
        assert_eq!(
            dcb._bitfield & RTS_CONTROL_ENABLE,
            RTS_CONTROL_ENABLE,
            "fRtsControl must stay enabled for EscapeCommFunction(RTS)"
        );
        assert_eq!(dcb._bitfield & F_OUTX_CTS_FLOW, F_OUTX_CTS_FLOW);
        assert_eq!(
            dcb._bitfield & F_PARITY,
            F_PARITY,
            "fParity follows parity mode"
        );

        // Switching to no parity clears fParity only.
        let nopar = SerialConfig { parity: 0, ..cfg };
        configure_dcb(&mut dcb, &nopar).expect("configure dcb without parity");
        assert_eq!(dcb._bitfield & F_PARITY, 0);
        assert_eq!(dcb._bitfield & DTR_CONTROL_ENABLE, DTR_CONTROL_ENABLE);
        assert_eq!(dcb.Parity, NOPARITY);
    }

    #[cfg(windows)]
    #[test]
    fn test_configure_dcb_maps_one_and_half_stop_bits() {
        let mut dcb: DCB = unsafe { std::mem::zeroed() };
        dcb.DCBlength = std::mem::size_of::<DCB>() as u32;
        let cfg = SerialConfig {
            baud_rate: 9_600,
            data_bits: 5,
            stop_bits: 3,
            parity: 1,
        };
        configure_dcb(&mut dcb, &cfg).expect("configure dcb");
        assert_eq!(dcb.StopBits, ONE5STOPBITS);
        assert_eq!(dcb.Parity, ODDPARITY);
    }

    #[tokio::test]
    async fn test_mock_uart_configuration_and_pins() {
        let config = SerialConfig {
            baud_rate: 9600,
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
        };

        let mut uart =
            Win32SerialEndpoint::open_mock(2, "uart2", "COM11", config).expect("open mock uart");

        assert_eq!(uart.current_config().baud_rate, 9600);
        assert_eq!(uart.throttler().byte_rate(), 960.0);

        // Reconfigure
        let new_cfg = SerialConfig {
            baud_rate: 57_600,
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
        };
        uart.configure(new_cfg).expect("configure");
        assert_eq!(uart.current_config().baud_rate, 57_600);
        assert_eq!(uart.throttler().byte_rate(), 5760.0);

        // Set pins
        uart.set_pins(PinState::DTR | PinState::RTS | PinState::BREAK)
            .expect("set pins");
        assert!(uart.pins.contains(PinState::DTR));
        assert!(uart.pins.contains(PinState::RTS));
        assert!(uart.pins.contains(PinState::BREAK));
    }
}
