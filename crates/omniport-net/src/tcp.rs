use bytes::Bytes;
use omniport_core::{
    AsyncEndpoint, EndpointId, FrameFlags, PinState, SerialConfig, SerialEvent,
};
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::rfc2217::{
    config_to_rfc2217_commands, modem_state_to_pin_state, pin_state_to_control, Rfc2217Codec,
    Rfc2217Command, COM_PORT_OPTION, DO, WILL,
};

pub struct TcpEndpoint {
    id: EndpointId,
    name: String,
    config: SerialConfig,
    pins: Arc<AtomicU8>,
    _outbound_tx: mpsc::Sender<SerialEvent>,
    outbound_rx: mpsc::Receiver<SerialEvent>,
    tcp_tx: mpsc::Sender<Bytes>,
    rfc2217_enabled: bool,
    _shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl TcpEndpoint {
    /// Creates a TCP client endpoint connecting to a remote address
    pub async fn connect_client(
        id: EndpointId,
        name: impl Into<String>,
        addr: SocketAddr,
        config: SerialConfig,
        rfc2217_enabled: bool,
    ) -> std::io::Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        Self::from_stream(id, name, stream, config, rfc2217_enabled)
    }

    /// Creates a TCP server endpoint accepting one active client
    pub async fn bind_server(
        id: EndpointId,
        name: impl Into<String>,
        bind_addr: SocketAddr,
        config: SerialConfig,
        rfc2217_enabled: bool,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(bind_addr).await?;
        let (stream, _) = listener.accept().await?;
        Self::from_stream(id, name, stream, config, rfc2217_enabled)
    }

    fn from_stream(
        id: EndpointId,
        name: impl Into<String>,
        stream: TcpStream,
        config: SerialConfig,
        rfc2217_enabled: bool,
    ) -> std::io::Result<Self> {
        let (mut read_half, mut write_half) = stream.into_split();
        let (outbound_tx, outbound_rx) = mpsc::channel(1024);
        let (tcp_tx, mut tcp_rx) = mpsc::channel::<Bytes>(1024);
        let pins = Arc::new(AtomicU8::new(PinState::empty().bits()));

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        // Background write loop
        tokio::spawn(async move {
            while let Some(bytes) = tcp_rx.recv().await {
                if write_half.write_all(&bytes).await.is_err() {
                    break;
                }
            }
        });

        // Initial RFC 2217 option negotiation if enabled
        if rfc2217_enabled {
            let init_negotiations = [
                Rfc2217Codec::encode_option(DO, COM_PORT_OPTION),
                Rfc2217Codec::encode_option(WILL, COM_PORT_OPTION),
            ];
            for neg in init_negotiations {
                let _ = tcp_tx.try_send(neg);
            }

            // Sync initial baud and port configuration
            for cmd in config_to_rfc2217_commands(&config) {
                let bytes = Rfc2217Codec::encode_command(&cmd);
                let _ = tcp_tx.try_send(bytes);
            }
        }

        // Background read loop
        let pins_clone = pins.clone();
        let out_tx = outbound_tx.clone();
        let source_id = id;

        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            let mut codec = Rfc2217Codec::new();

            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    read_res = read_half.read(&mut buf) => {
                        let n = match read_res {
                            Ok(0) => break, // EOF
                            Ok(n) => n,
                            Err(_) => break,
                        };

                        let now_ns = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos() as u64;

                        if rfc2217_enabled {
                            let (clean_data, commands, _) = codec.decode_chunk(&buf[..n]);

                            // Handle RFC 2217 in-band control commands
                            for cmd in commands {
                                if let Rfc2217Command::NotifyModemState(ms) = cmd {
                                    let current = PinState::from_bits_truncate(pins_clone.load(Ordering::Acquire));
                                    let updated = modem_state_to_pin_state(ms, current);
                                    pins_clone.store(updated.bits(), Ordering::Release);

                                    let pin_evt = SerialEvent {
                                        timestamp_ns: now_ns,
                                        source_id,
                                        flags: FrameFlags::PIN_CHANGE,
                                        pins: updated,
                                        payload: Bytes::new(),
                                    };
                                    let _ = out_tx.send(pin_evt).await;
                                }
                            }

                            if !clean_data.is_empty() {
                                let cur_pins = PinState::from_bits_truncate(pins_clone.load(Ordering::Acquire));
                                let evt = SerialEvent {
                                    timestamp_ns: now_ns,
                                    source_id,
                                    flags: FrameFlags::empty(),
                                    pins: cur_pins,
                                    payload: Bytes::copy_from_slice(&clean_data),
                                };
                                if out_tx.send(evt).await.is_err() {
                                    break;
                                }
                            }
                        } else {
                            // Raw TCP streaming
                            let cur_pins = PinState::from_bits_truncate(pins_clone.load(Ordering::Acquire));
                            let evt = SerialEvent {
                                timestamp_ns: now_ns,
                                source_id,
                                flags: FrameFlags::empty(),
                                pins: cur_pins,
                                payload: Bytes::copy_from_slice(&buf[..n]),
                            };
                            if out_tx.send(evt).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            id,
            name: name.into(),
            config,
            pins,
            _outbound_tx: outbound_tx,
            outbound_rx,
            tcp_tx,
            rfc2217_enabled,
            _shutdown_tx: Some(shutdown_tx),
        })
    }
}

impl AsyncEndpoint for TcpEndpoint {
    fn id(&self) -> EndpointId {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn recv(&mut self) -> Pin<Box<dyn Future<Output = std::io::Result<SerialEvent>> + Send + '_>> {
        Box::pin(async move {
            match self.outbound_rx.recv().await {
                Some(evt) => Ok(evt),
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "TCP endpoint closed",
                )),
            }
        })
    }

    fn send(
        &mut self,
        event: SerialEvent,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send + '_>> {
        Box::pin(async move {
            let data_to_send = if self.rfc2217_enabled {
                Rfc2217Codec::encode_data(&event.payload)
            } else {
                event.payload
            };

            self.tcp_tx
                .send(data_to_send)
                .await
                .map_err(|_| std::io::Error::other("TCP write queue closed"))?;

            Ok(())
        })
    }

    fn set_pins(&mut self, new_pins: PinState) -> std::io::Result<()> {
        let old_pins = PinState::from_bits_truncate(self.pins.swap(new_pins.bits(), Ordering::AcqRel));

        if self.rfc2217_enabled {
            let commands = pin_state_to_control(new_pins, old_pins);
            for cmd in commands {
                let bytes = Rfc2217Codec::encode_command(&cmd);
                let _ = self.tcp_tx.try_send(bytes);
            }
        }

        Ok(())
    }

    fn configure(&mut self, cfg: SerialConfig) -> std::io::Result<()> {
        self.config = cfg;

        if self.rfc2217_enabled {
            for cmd in config_to_rfc2217_commands(&cfg) {
                let bytes = Rfc2217Codec::encode_command(&cmd);
                let _ = self.tcp_tx.try_send(bytes);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tcp_raw_loopback_roundtrip() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server_task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            TcpEndpoint::from_stream(10, "Server", stream, SerialConfig::default(), false).unwrap()
        });

        let mut client = TcpEndpoint::connect_client(
            11,
            "Client",
            addr,
            SerialConfig::default(),
            false,
        )
        .await
        .unwrap();

        let mut server = server_task.await.unwrap();

        // Client -> Server
        let send_evt = SerialEvent {
            timestamp_ns: 12345,
            source_id: 11,
            flags: FrameFlags::empty(),
            pins: PinState::empty(),
            payload: Bytes::copy_from_slice(b"NET_PING"),
        };
        client.send(send_evt).await.unwrap();

        let recv_evt = server.recv().await.unwrap();
        assert_eq!(&recv_evt.payload[..], b"NET_PING");
    }

    #[tokio::test]
    async fn test_tcp_rfc2217_modem_pin_sync() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server_task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();

            // Server reads client's option negotiation
            let mut buf = [0u8; 128];
            let _ = stream.read(&mut buf).await.unwrap();

            // Server sends back RFC 2217 NOTIFY_MODEMSTATE with CTS active (0x10)
            let notify_cmd = Rfc2217Command::NotifyModemState(0x10);
            let bytes = Rfc2217Codec::encode_command(&notify_cmd);
            stream.write_all(&bytes).await.unwrap();

            // Keep connection open briefly
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        });

        let mut client = TcpEndpoint::connect_client(
            20,
            "RfcClient",
            addr,
            SerialConfig::default(),
            true,
        )
        .await
        .unwrap();

        // Await PIN_CHANGE event from server
        let pin_evt = client.recv().await.unwrap();
        assert!(pin_evt.flags.contains(FrameFlags::PIN_CHANGE));
        assert!(pin_evt.pins.contains(PinState::CTS));

        server_task.await.unwrap();
    }
}
