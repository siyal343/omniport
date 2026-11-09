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
                    tokio::task::spawn_blocking(move || {
                        let mut bytes_read = 0u32;
                        let mut overlapped: windows_sys::Win32::System::IO::OVERLAPPED =
                            unsafe { std::mem::zeroed() };
                        let event = unsafe {
                            CreateEventW(std::ptr::null(), TRUE, FALSE, std::ptr::null())
                        };
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
                            if err == ERROR_IO_PENDING {
                                unsafe {
                                    WaitForSingleObject(event, INFINITE);
                                    windows_sys::Win32::System::IO::GetOverlappedResult(
                                        h.0,
                                        &overlapped,
                                        &mut bytes_read,
                                        TRUE,
                                    );
                                }
                            }
                        }
                        unsafe { CloseHandle(event) };
                        (lbuf, bytes_read as usize)
                    })
                    .await
                    .map(|(lbuf, cnt)| {
                        buf[..cnt].copy_from_slice(&lbuf[..cnt]);
                        cnt
                    })
                    .map_err(std::io::Error::other)?
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
                                if err == ERROR_IO_PENDING {
                                    unsafe {
                                        WaitForSingleObject(event, INFINITE);
                                        windows_sys::Win32::System::IO::GetOverlappedResult(
                                            h.0,
                                            &overlapped,
                                            &mut bytes_written,
                                            TRUE,
                                        );
                                    }
                                }
                            }
                            unsafe { CloseHandle(event) };
                        })
                        .await
                        .map_err(std::io::Error::other)?;
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
    let mut dcb: DCB = unsafe { std::mem::zeroed() };
    dcb.DCBlength = std::mem::size_of::<DCB>() as u32;

    if unsafe { GetCommState(handle, &mut dcb) } == 0 {
        return Err(HalError::CommState(std::io::Error::last_os_error()));
    }

    dcb.BaudRate = cfg.baud_rate;
    dcb.ByteSize = cfg.data_bits;

    dcb.StopBits = match cfg.stop_bits {
        2 => TWOSTOPBITS,
        _ => ONESTOPBIT,
    };

    dcb.Parity = match cfg.parity {
        1 => ODDPARITY,
        2 => EVENPARITY,
        _ => NOPARITY,
    };

    // Set binary mode
    dcb._bitfield = 1; // fBinary = 1

    if unsafe { SetCommState(handle, &dcb) } == 0 {
        return Err(HalError::CommState(std::io::Error::last_os_error()));
    }
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
