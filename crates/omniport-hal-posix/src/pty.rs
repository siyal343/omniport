#[cfg(unix)]
use crate::error::HalError;
use crate::error::Result;
use crate::symlink::{SymlinkGuard, SymlinkManager};
use crate::termios::TermiosPoller;
use crate::throttler::TokenBucketThrottler;
use bytes::Bytes;
use omniport_core::{AsyncEndpoint, EndpointId, FrameFlags, PinState, SerialConfig, SerialEvent};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

#[cfg(unix)]
use std::os::fd::AsRawFd;

enum PtyIo {
    #[cfg(unix)]
    Unix {
        async_fd: tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>,
        _slave_fd: Option<std::os::fd::OwnedFd>,
    },
    #[cfg(not(unix))]
    Mock {
        master: tokio::io::DuplexStream,
        slave: Option<tokio::io::DuplexStream>,
    },
}

impl PtyIo {
    async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            PtyIo::Unix { async_fd, .. } => loop {
                let mut guard = async_fd.readable().await?;
                match guard.try_io(|inner| {
                    let res = unsafe {
                        libc::read(
                            inner.as_raw_fd(),
                            buf.as_mut_ptr() as *mut libc::c_void,
                            buf.len(),
                        )
                    };
                    if res < 0 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(res as usize)
                    }
                }) {
                    Ok(res) => return res,
                    Err(_would_block) => continue,
                }
            },
            #[cfg(not(unix))]
            PtyIo::Mock { master, .. } => {
                use tokio::io::AsyncReadExt;
                let n = master.read(buf).await?;
                if n == 0 {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "PTY slave disconnected",
                    ))
                } else {
                    Ok(n)
                }
            }
        }
    }

    async fn write_all(&mut self, #[allow(unused_mut)] mut buf: &[u8]) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            PtyIo::Unix { async_fd, .. } => {
                while !buf.is_empty() {
                    let mut guard = async_fd.writable().await?;
                    match guard.try_io(|inner| {
                        let res = unsafe {
                            libc::write(
                                inner.as_raw_fd(),
                                buf.as_ptr() as *const libc::c_void,
                                buf.len(),
                            )
                        };
                        if res < 0 {
                            Err(std::io::Error::last_os_error())
                        } else {
                            Ok(res as usize)
                        }
                    }) {
                        Ok(Ok(n)) => {
                            buf = &buf[n..];
                        }
                        Ok(Err(e)) => return Err(e),
                        Err(_would_block) => continue,
                    }
                }
                Ok(())
            }
            #[cfg(not(unix))]
            PtyIo::Mock { master, .. } => {
                use tokio::io::AsyncWriteExt;
                master.write_all(buf).await
            }
        }
    }
}

/// Tier-1 POSIX Pseudo-Terminal (PTY) Endpoint with software baud rate throttling
pub struct PtyEndpoint {
    id: EndpointId,
    name: String,
    slave_path: PathBuf,
    symlink: Option<SymlinkGuard>,
    throttler: TokenBucketThrottler,
    poller: TermiosPoller,
    pins: PinState,
    config: SerialConfig,
    io: PtyIo,
}

impl PtyEndpoint {
    /// Opens a new PTY pair and optionally creates a symbolic link pointing to the slave device
    pub fn open(
        id: EndpointId,
        name: impl Into<String>,
        config: SerialConfig,
        symlink_path: Option<&Path>,
    ) -> Result<Self> {
        let name_str = name.into();

        #[cfg(unix)]
        {
            let (master_fd, slave_path, slave_fd) = open_unix_pty()?;
            let poller = TermiosPoller::new(slave_fd.as_ref().map(|f| f.as_raw_fd()), config);
            let async_fd = tokio::io::unix::AsyncFd::new(master_fd).map_err(|e| HalError::Io(e))?;

            let symlink = if let Some(link) = symlink_path {
                Some(SymlinkManager::create(&slave_path, link)?)
            } else {
                None
            };

            let throttler = TokenBucketThrottler::new(config);
            Ok(Self {
                id,
                name: name_str,
                slave_path,
                symlink,
                throttler,
                poller,
                pins: PinState::empty(),
                config,
                io: PtyIo::Unix {
                    async_fd,
                    _slave_fd: slave_fd,
                },
            })
        }

        #[cfg(not(unix))]
        {
            let (master, slave) = tokio::io::duplex(65536);
            let slave_path = PathBuf::from(format!("mock://pts/{}", id));
            let poller = TermiosPoller::new(None, config);

            let symlink = if let Some(link) = symlink_path {
                Some(SymlinkManager::create(&slave_path, link)?)
            } else {
                None
            };

            let throttler = TokenBucketThrottler::new(config);
            Ok(Self {
                id,
                name: name_str,
                slave_path,
                symlink,
                throttler,
                poller,
                pins: PinState::empty(),
                config,
                io: PtyIo::Mock {
                    master,
                    slave: Some(slave),
                },
            })
        }
    }

    pub fn slave_path(&self) -> &Path {
        &self.slave_path
    }

    pub fn symlink_path(&self) -> Option<&Path> {
        self.symlink.as_ref().map(|g| g.link_path())
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

    pub fn poller_mut(&mut self) -> &mut TermiosPoller {
        &mut self.poller
    }

    /// Checks if the client application updated slave serial port parameters via termios
    pub fn poll_termios(&mut self) -> Result<Option<SerialConfig>> {
        if let Some(new_cfg) = self.poller.poll()? {
            self.config = new_cfg;
            self.throttler.update_config(new_cfg);
            Ok(Some(new_cfg))
        } else {
            Ok(None)
        }
    }

    /// In non-Unix testing environments, extracts the mock slave stream for end-to-end testing
    #[cfg(not(unix))]
    pub fn take_mock_slave(&mut self) -> Option<tokio::io::DuplexStream> {
        match &mut self.io {
            PtyIo::Mock { slave, .. } => slave.take(),
        }
    }
}

impl AsyncEndpoint for PtyEndpoint {
    fn id(&self) -> EndpointId {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn recv(&mut self) -> Pin<Box<dyn Future<Output = std::io::Result<SerialEvent>> + Send + '_>> {
        Box::pin(async move {
            // Check for dynamic baud rate changes from client termios
            let _ = self.poll_termios();

            let mut buf = [0u8; 4096];
            let n = self.io.read(&mut buf).await?;

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
                self.io.write_all(&event.payload).await?;
            }
            Ok(())
        })
    }

    fn set_pins(&mut self, pins: PinState) -> std::io::Result<()> {
        self.pins = pins;
        Ok(())
    }

    fn configure(&mut self, cfg: SerialConfig) -> std::io::Result<()> {
        self.config = cfg;
        self.throttler.update_config(cfg);
        Ok(())
    }
}

#[cfg(unix)]
fn open_unix_pty() -> Result<(std::os::fd::OwnedFd, PathBuf, Option<std::os::fd::OwnedFd>)> {
    use std::os::fd::FromRawFd;

    let master_fd = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK) };
    if master_fd < 0 {
        return Err(HalError::OpenPt(std::io::Error::last_os_error()));
    }

    if unsafe { libc::grantpt(master_fd) } != 0 {
        let err = std::io::Error::last_os_error();
        unsafe { libc::close(master_fd) };
        return Err(HalError::GrantPt(err));
    }

    if unsafe { libc::unlockpt(master_fd) } != 0 {
        let err = std::io::Error::last_os_error();
        unsafe { libc::close(master_fd) };
        return Err(HalError::UnlockPt(err));
    }

    let mut buf = [0u8; 256];
    let res =
        unsafe { libc::ptsname_r(master_fd, buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if res != 0 {
        let err = std::io::Error::last_os_error();
        unsafe { libc::close(master_fd) };
        return Err(HalError::PtsName(err.to_string()));
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let pts_str = std::str::from_utf8(&buf[..len]).map_err(|e| HalError::PtsName(e.to_string()))?;
    let slave_path = PathBuf::from(pts_str);

    // Open slave in non-blocking mode to configure raw mode and for termios polling
    let c_path = std::ffi::CString::new(pts_str).map_err(|e| HalError::PtsName(e.to_string()))?;
    let slave_fd = unsafe {
        libc::open(
            c_path.as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK,
        )
    };
    let slave_owned = if slave_fd >= 0 {
        let _ = crate::termios::set_raw_mode(slave_fd);
        Some(unsafe { std::os::fd::OwnedFd::from_raw_fd(slave_fd) })
    } else {
        None
    };

    let owned_master = unsafe { std::os::fd::OwnedFd::from_raw_fd(master_fd) };
    Ok((owned_master, slave_path, slave_owned))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn test_pty_endpoint_lifecycle_and_io() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let symlink_path = temp_dir.path().join("vport1");

        let config = SerialConfig {
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
        };

        let mut pty =
            PtyEndpoint::open(42, "ttyV0", config, Some(&symlink_path)).expect("open pty endpoint");

        assert_eq!(pty.id(), 42);
        assert_eq!(pty.name(), "ttyV0");
        assert_eq!(pty.symlink_path(), Some(symlink_path.as_path()));
        assert!(symlink_path.exists());

        #[cfg(not(unix))]
        {
            let mut slave = pty.take_mock_slave().expect("mock slave stream");

            // Test 1: Send from PTY endpoint to slave
            let send_event = SerialEvent {
                timestamp_ns: 1000,
                source_id: 42,
                flags: FrameFlags::empty(),
                pins: PinState::RTS,
                payload: Bytes::from_static(b"AT+GMR\r\n"),
            };
            pty.send(send_event).await.expect("send to slave");

            let mut slave_buf = vec![0u8; 8];
            slave.read_exact(&mut slave_buf).await.expect("read slave");
            assert_eq!(&slave_buf[..], b"AT+GMR\r\n");

            // Test 2: Send from slave to PTY endpoint (recv)
            slave
                .write_all(b"OmniPort-v1.0\r\n")
                .await
                .expect("write from slave");

            let recv_event = pty.recv().await.expect("recv event");
            assert_eq!(recv_event.source_id, 42);
            assert_eq!(&recv_event.payload[..], b"OmniPort-v1.0\r\n");

            // Test 3: Pin state
            pty.set_pins(PinState::DTR | PinState::RTS)
                .expect("set pins");
            assert_eq!(pty.pins, PinState::DTR | PinState::RTS);
        }

        // Test 4: Symlink cleanup on drop
        drop(pty);
        assert!(!symlink_path.exists());
    }

    #[tokio::test]
    async fn test_pty_endpoint_termios_reconfiguration() {
        let config = SerialConfig {
            baud_rate: 9600,
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
        };

        let mut pty = PtyEndpoint::open(1, "ttyV1", config, None).expect("open pty");
        assert_eq!(pty.current_config().baud_rate, 9600);
        assert_eq!(pty.throttler().byte_rate(), 960.0);

        // Reconfigure programmatically
        let new_cfg = SerialConfig {
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
        };
        pty.configure(new_cfg).expect("configure");
        assert_eq!(pty.current_config().baud_rate, 115_200);
        assert_eq!(pty.throttler().byte_rate(), 11_520.0);
    }
}
