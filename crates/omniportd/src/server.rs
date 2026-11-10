//! Control-plane RPC server with bounded connections and timeouts.
//!
//! `omniportd` listens on loopback by default. Every connection is subject to
//! an idle deadline, a per-request processing deadline, and an optional request
//! rate limit; total concurrency is bounded by a semaphore. Over-limit clients
//! get an explicit protocol error (`Response::Error` 429/503) instead of a hang.
//!
//! Idle connections are closed silently rather than receiving an unsolicited
//! `408` frame: the control plane is strictly request/response, so a frame the
//! client did not ask for would be mis-read as the response to its next
//! request.

use crate::daemon::DaemonEngine;
use omniport_proto::codec::{read_message, write_message};
use omniport_proto::{ProtoError, Request, Response};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;

/// Default control-plane bind address (loopback only).
pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:50051";
/// Default maximum concurrent client connections.
pub const DEFAULT_MAX_CONNECTIONS: usize = 64;
/// Default idle deadline for a connected client.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Default deadline for handling a single request.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Default per-connection request rate limit (requests per second); 0 disables.
pub const DEFAULT_MAX_REQUESTS_PER_SECOND: u32 = 200;
/// How many overloaded-connection rejections may be in flight at once.
const REJECT_BURST: usize = 16;
/// Bound on startup/shutdown drain of connection tasks.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

/// Runtime policy for the control-plane server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub bind_addr: String,
    pub max_connections: usize,
    pub idle_timeout: Duration,
    pub request_timeout: Duration,
    /// Per-connection request rate limit; `0` disables the limit.
    pub max_requests_per_second: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: DEFAULT_BIND_ADDR.to_string(),
            max_connections: DEFAULT_MAX_CONNECTIONS,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            max_requests_per_second: DEFAULT_MAX_REQUESTS_PER_SECOND,
        }
    }
}

/// True when `addr` binds only to a loopback interface (`127.0.0.0/8`, `::1`
/// or `localhost`).
pub fn bind_is_loopback(addr: &str) -> bool {
    if let Ok(socket_addr) = addr.parse::<SocketAddr>() {
        return socket_addr.ip().is_loopback();
    }
    match addr.rsplit_once(':') {
        Some((host, _port)) => {
            let host = host.trim_start_matches('[').trim_end_matches(']');
            host.eq_ignore_ascii_case("localhost")
        }
        None => false,
    }
}

impl ServerConfig {
    /// Validates the configuration before the listener binds.
    ///
    /// Remote control is unauthenticated (loopback TCP has no peer-credential
    /// checks yet), so binding a non-loopback interface requires the explicit
    /// `--allow-remote` opt-in.
    pub fn validate(&self, allow_remote: bool) -> Result<(), String> {
        if self.bind_addr.trim().is_empty() {
            return Err("bind address cannot be empty".to_string());
        }
        if self.max_connections == 0 {
            return Err("max_connections must be at least 1".to_string());
        }
        if self.idle_timeout.is_zero() {
            return Err("idle timeout must be greater than zero".to_string());
        }
        if self.request_timeout.is_zero() {
            return Err("request timeout must be greater than zero".to_string());
        }
        if !allow_remote && !bind_is_loopback(&self.bind_addr) {
            return Err(format!(
                "refusing to bind non-loopback address {} without --allow-remote: the control plane has no authentication",
                self.bind_addr
            ));
        }
        Ok(())
    }
}

/// Runs the control-plane RPC server with default limits.
pub async fn run_server(
    addr: &str,
    engine: Arc<DaemonEngine>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = ServerConfig {
        bind_addr: addr.to_string(),
        ..ServerConfig::default()
    };
    run_server_with_config(config, engine).await
}

/// Runs the control-plane RPC server with the given policy.
pub async fn run_server_with_config(
    config: ServerConfig,
    engine: Arc<DaemonEngine>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind(&config.bind_addr).await?;
    let mut shutdown_rx = engine.subscribe_shutdown();
    let mut connections = JoinSet::new();
    let config = Arc::new(config);
    let permits = Arc::new(Semaphore::new(config.max_connections));
    let reject_permits = Arc::new(Semaphore::new(REJECT_BURST));

    loop {
        tokio::select! {
            res = listener.accept() => {
                let (socket, peer_addr) = match res {
                    Ok(pair) => pair,
                    Err(e) => {
                        eprintln!("omniportd accept error: {}", e);
                        continue;
                    }
                };

                while connections.try_join_next().is_some() {}

                match Arc::clone(&permits).try_acquire_owned() {
                    Ok(permit) => {
                        let eng = Arc::clone(&engine);
                        let cfg = Arc::clone(&config);
                        connections.spawn(async move {
                            handle_connection(socket, eng, cfg, permit, peer_addr).await;
                        });
                    }
                    Err(_) => {
                        eprintln!(
                            "omniportd rejecting connection from {}: connection limit {} reached",
                            peer_addr, config.max_connections
                        );
                        if let Ok(reject_permit) = Arc::clone(&reject_permits).try_acquire_owned() {
                            let eng = Arc::clone(&engine);
                            let cfg = Arc::clone(&config);
                            connections.spawn(async move {
                                reject_overloaded(socket, eng, cfg, reject_permit, peer_addr).await;
                            });
                        }
                        // Without a reject slot the socket is dropped immediately.
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                break;
            }
        }
    }

    drop(listener);
    let _ = tokio::time::timeout(DRAIN_TIMEOUT, async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    connections.abort_all();

    Ok(())
}

/// Services one admitted client connection until it closes, goes idle, or the
/// daemon shuts down.
async fn handle_connection(
    mut socket: TcpStream,
    engine: Arc<DaemonEngine>,
    config: Arc<ServerConfig>,
    _permit: OwnedSemaphorePermit,
    peer_addr: SocketAddr,
) {
    let mut conn_shutdown_rx = engine.subscribe_shutdown();
    let mut window_start = Instant::now();
    let mut requests_in_window: u32 = 0;

    loop {
        if engine.is_shutting_down.load(Ordering::SeqCst) {
            break;
        }

        let req: Request = tokio::select! {
            biased;
            _ = conn_shutdown_rx.recv() => break,
            msg = tokio::time::timeout(config.idle_timeout, read_message(&mut socket)) => {
                match msg {
                    // Idle: close silently (see module docs on unsolicited frames).
                    Err(_) => {
                        eprintln!(
                            "omniportd closing idle connection from {} after {:?}",
                            peer_addr, config.idle_timeout
                        );
                        break;
                    }
                    Ok(Ok(msg)) => msg,
                    Ok(Err(ProtoError::ConnectionClosed)) => break,
                    Ok(Err(ProtoError::FrameTooLarge { size, max })) => {
                        eprintln!(
                            "omniportd closing connection from {}: frame of {} bytes exceeds {}",
                            peer_addr, size, max
                        );
                        break;
                    }
                    Ok(Err(e)) => {
                        eprintln!("RPC read error from {}: {}", peer_addr, e);
                        break;
                    }
                }
            }
        };

        if config.max_requests_per_second > 0 {
            if window_start.elapsed() >= Duration::from_secs(1) {
                window_start = Instant::now();
                requests_in_window = 0;
            }
            requests_in_window = requests_in_window.saturating_add(1);
            if requests_in_window > config.max_requests_per_second {
                eprintln!(
                    "omniportd rate-limiting connection from {}: {} requests in one second",
                    peer_addr, requests_in_window
                );
                let _ = write_message(
                    &mut socket,
                    &Response::Error {
                        code: 429,
                        message: format!(
                            "request rate limit exceeded ({} requests/second per connection)",
                            config.max_requests_per_second
                        ),
                    },
                )
                .await;
                break;
            }
        }

        let resp: Response =
            match tokio::time::timeout(config.request_timeout, engine.handle_request(req)).await {
                Ok(resp) => resp,
                Err(_) => Response::Error {
                    code: 503,
                    message: format!("request timed out after {:?}", config.request_timeout),
                },
            };

        if let Err(e) = write_message(&mut socket, &resp).await {
            eprintln!("RPC write error to {}: {}", peer_addr, e);
            break;
        }
    }

    let _ = socket.shutdown().await;
}

/// Handles a connection accepted while the server is at its connection limit:
/// reads one request (bounded) and answers it with an explicit 503 before
/// closing. Connections that never send a request are closed silently.
async fn reject_overloaded(
    mut socket: TcpStream,
    engine: Arc<DaemonEngine>,
    config: Arc<ServerConfig>,
    _reject_permit: OwnedSemaphorePermit,
    peer_addr: SocketAddr,
) {
    let mut shutdown_rx = engine.subscribe_shutdown();
    let read_deadline = config.request_timeout.min(Duration::from_secs(2));
    let request_seen = tokio::select! {
        biased;
        _ = shutdown_rx.recv() => false,
        result = tokio::time::timeout(read_deadline, read_message::<_, Request>(&mut socket)) => {
            matches!(result, Ok(Ok(_)))
        }
    };

    if request_seen {
        let _ = write_message(
            &mut socket,
            &Response::Error {
                code: 503,
                message: format!(
                    "connection limit reached ({} concurrent clients)",
                    config.max_connections
                ),
            },
        )
        .await;
    }
    let _ = socket.shutdown().await;
    eprintln!(
        "omniportd rejected overloaded connection from {}",
        peer_addr
    );
}

/// Re-exported so callers can size their buffers against the same limit.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_is_loopback_and_valid() {
        let config = ServerConfig::default();
        assert_eq!(config.bind_addr, DEFAULT_BIND_ADDR);
        assert!(config.validate(false).is_ok());
        assert!(bind_is_loopback(&config.bind_addr));
    }

    #[test]
    fn test_bind_is_loopback_recognises_loopback_forms() {
        assert!(bind_is_loopback("127.0.0.1:50051"));
        assert!(bind_is_loopback("127.1.2.3:9"));
        assert!(bind_is_loopback("[::1]:50051"));
        assert!(bind_is_loopback("localhost:50051"));
        assert!(!bind_is_loopback("0.0.0.0:50051"));
        assert!(!bind_is_loopback("[::]:50051"));
        assert!(!bind_is_loopback("192.168.1.10:50051"));
        assert!(!bind_is_loopback("example.com:50051"));
    }

    #[test]
    fn test_remote_bind_requires_explicit_opt_in() {
        let config = ServerConfig {
            bind_addr: "0.0.0.0:50051".to_string(),
            ..ServerConfig::default()
        };
        let err = config
            .validate(false)
            .expect_err("remote bind must be refused");
        assert!(err.contains("--allow-remote"));
        assert!(config.validate(true).is_ok());
    }

    #[test]
    fn test_invalid_limits_are_rejected() {
        assert!(
            ServerConfig {
                max_connections: 0,
                ..ServerConfig::default()
            }
            .validate(false)
            .is_err()
        );
        assert!(
            ServerConfig {
                idle_timeout: Duration::ZERO,
                ..ServerConfig::default()
            }
            .validate(false)
            .is_err()
        );
        assert!(
            ServerConfig {
                request_timeout: Duration::ZERO,
                ..ServerConfig::default()
            }
            .validate(false)
            .is_err()
        );
        assert!(
            ServerConfig {
                bind_addr: "   ".to_string(),
                ..ServerConfig::default()
            }
            .validate(false)
            .is_err()
        );
    }
}
