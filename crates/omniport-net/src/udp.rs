use bytes::Bytes;
use omniport_core::{AsyncEndpoint, EndpointId, FrameFlags, PinState, SerialConfig, SerialEvent};
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

pub struct UdpEndpoint {
    id: EndpointId,
    name: String,
    config: SerialConfig,
    pins: Arc<AtomicU8>,
    socket: Arc<UdpSocket>,
    target_addr: SocketAddr,
    outbound_rx: mpsc::Receiver<SerialEvent>,
    _shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl UdpEndpoint {
    pub async fn bind(
        id: EndpointId,
        name: impl Into<String>,
        bind_addr: SocketAddr,
        target_addr: SocketAddr,
        config: SerialConfig,
    ) -> std::io::Result<Self> {
        let socket = Arc::new(UdpSocket::bind(bind_addr).await?);
        let pins = Arc::new(AtomicU8::new(PinState::empty().bits()));
        let (outbound_tx, outbound_rx) = mpsc::channel(1024);
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let socket_recv = socket.clone();
        let pins_clone = pins.clone();
        let source_id = id;

        // Background UDP listener loop
        tokio::spawn(async move {
            let mut buf = [0u8; 65535];
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    res = socket_recv.recv_from(&mut buf) => {
                        match res {
                            Ok((len, _from)) => {
                                let now_ns = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_nanos() as u64;

                                let cur_pins = PinState::from_bits_truncate(pins_clone.load(Ordering::Acquire));
                                let evt = SerialEvent {
                                    timestamp_ns: now_ns,
                                    source_id,
                                    flags: FrameFlags::empty(),
                                    pins: cur_pins,
                                    payload: Bytes::copy_from_slice(&buf[..len]),
                                };
                                if outbound_tx.send(evt).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
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
            socket,
            target_addr,
            outbound_rx,
            _shutdown_tx: Some(shutdown_tx),
        })
    }
}

impl AsyncEndpoint for UdpEndpoint {
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
                    "UDP endpoint closed",
                )),
            }
        })
    }

    fn send(
        &mut self,
        event: SerialEvent,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send + '_>> {
        let socket = self.socket.clone();
        let target = self.target_addr;
        Box::pin(async move {
            socket.send_to(&event.payload, target).await?;
            Ok(())
        })
    }

    fn set_pins(&mut self, pins: PinState) -> std::io::Result<()> {
        self.pins.store(pins.bits(), Ordering::Release);
        Ok(())
    }

    fn configure(&mut self, cfg: SerialConfig) -> std::io::Result<()> {
        self.config = cfg;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_udp_bidirectional_loopback() {
        let mut node_a = UdpEndpoint::bind(
            1,
            "NodeA",
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.1:0".parse().unwrap(), // placeholder
            SerialConfig::default(),
        )
        .await
        .unwrap();

        let addr_a = node_a.socket.local_addr().unwrap();

        let mut node_b = UdpEndpoint::bind(
            2,
            "NodeB",
            "127.0.0.1:0".parse().unwrap(),
            addr_a,
            SerialConfig::default(),
        )
        .await
        .unwrap();

        node_a.target_addr = node_b.socket.local_addr().unwrap();

        // Node A -> Node B
        let send_evt = SerialEvent {
            timestamp_ns: 100,
            source_id: 1,
            flags: FrameFlags::empty(),
            pins: PinState::empty(),
            payload: Bytes::copy_from_slice(b"UDP_HELLO"),
        };
        node_a.send(send_evt).await.unwrap();

        let recv_evt = node_b.recv().await.unwrap();
        assert_eq!(&recv_evt.payload[..], b"UDP_HELLO");
    }
}
