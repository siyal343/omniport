use bytes::Bytes;
use omniport_core::{
    AsyncEndpoint, EndpointId, FrameFlags, PinState, SerialConfig, SerialEvent,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::server::WebPkiClientVerifier;
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_rustls::{TlsAcceptor, TlsConnector, TlsStream};

pub struct TlsEndpoint {
    id: EndpointId,
    name: String,
    config: SerialConfig,
    pins: Arc<AtomicU8>,
    outbound_rx: mpsc::Receiver<SerialEvent>,
    tls_tx: mpsc::Sender<Bytes>,
    _shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl TlsEndpoint {
    /// Connects to a remote TLS server using mutual authentication (mTLS)
    #[allow(clippy::too_many_arguments)]
    pub async fn connect_client(
        id: EndpointId,
        name: impl Into<String>,
        addr: SocketAddr,
        server_name: ServerName<'static>,
        ca_cert: CertificateDer<'static>,
        client_cert: Vec<CertificateDer<'static>>,
        client_key: PrivateKeyDer<'static>,
        config: SerialConfig,
    ) -> std::io::Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();

        let mut root_store = RootCertStore::empty();
        root_store
            .add(ca_cert)
            .map_err(|e| std::io::Error::other(format!("Failed to add CA cert: {e}")))?;

        let tls_config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_client_auth_cert(client_cert, client_key)
            .map_err(|e| std::io::Error::other(format!("Invalid mTLS client config: {e}")))?;

        let connector = TlsConnector::from(Arc::new(tls_config));
        let tcp = TcpStream::connect(addr).await?;
        let tls_stream = connector
            .connect(server_name, tcp)
            .await
            .map_err(|e| std::io::Error::other(format!("TLS handshake failed: {e}")))?;

        Self::from_tls_stream(id, name, TlsStream::Client(tls_stream), config)
    }

    /// Accepts an incoming connection and performs mTLS handshake requiring client certificate
    pub async fn accept_server(
        id: EndpointId,
        name: impl Into<String>,
        listener: &TcpListener,
        ca_cert: CertificateDer<'static>,
        server_cert: Vec<CertificateDer<'static>>,
        server_key: PrivateKeyDer<'static>,
        config: SerialConfig,
    ) -> std::io::Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();

        let mut root_store = RootCertStore::empty();
        root_store
            .add(ca_cert)
            .map_err(|e| std::io::Error::other(format!("Failed to add client CA: {e}")))?;

        let client_verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
            .build()
            .map_err(|e| std::io::Error::other(format!("Verifier error: {e}")))?;

        let tls_config = ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(server_cert, server_key)
            .map_err(|e| std::io::Error::other(format!("Invalid mTLS server config: {e}")))?;

        let acceptor = TlsAcceptor::from(Arc::new(tls_config));
        let (tcp, _) = listener.accept().await?;
        let tls_stream = acceptor
            .accept(tcp)
            .await
            .map_err(|e| std::io::Error::other(format!("Server TLS handshake failed: {e}")))?;

        Self::from_tls_stream(id, name, TlsStream::Server(tls_stream), config)
    }

    fn from_tls_stream(
        id: EndpointId,
        name: impl Into<String>,
        stream: TlsStream<TcpStream>,
        config: SerialConfig,
    ) -> std::io::Result<Self> {
        let (mut read_half, mut write_half) = tokio::io::split(stream);
        let (outbound_tx, outbound_rx) = mpsc::channel(1024);
        let (tls_tx, mut tls_rx) = mpsc::channel::<Bytes>(1024);
        let pins = Arc::new(AtomicU8::new(PinState::empty().bits()));
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        // Background write loop
        tokio::spawn(async move {
            while let Some(bytes) = tls_rx.recv().await {
                if write_half.write_all(&bytes).await.is_err() {
                    break;
                }
            }
        });

        // Background read loop
        let pins_clone = pins.clone();
        let source_id = id;

        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    read_res = read_half.read(&mut buf) => {
                        match read_res {
                            Ok(0) => break, // EOF
                            Ok(n) => {
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
                                    payload: Bytes::copy_from_slice(&buf[..n]),
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
            outbound_rx,
            tls_tx,
            _shutdown_tx: Some(shutdown_tx),
        })
    }
}

impl AsyncEndpoint for TlsEndpoint {
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
                    "mTLS endpoint closed",
                )),
            }
        })
    }

    fn send(
        &mut self,
        event: SerialEvent,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send + '_>> {
        Box::pin(async move {
            self.tls_tx
                .send(event.payload)
                .await
                .map_err(|_| std::io::Error::other("TLS write queue closed"))?;
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
pub mod test_helpers {
    use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    pub fn generate_test_mtls_certs() -> (
        CertificateDer<'static>,
        Vec<CertificateDer<'static>>,
        PrivateKeyDer<'static>,
        Vec<CertificateDer<'static>>,
        PrivateKeyDer<'static>,
    ) {
        // 1. Generate Root CA
        let mut ca_params = CertificateParams::default();
        let mut ca_dn = DistinguishedName::new();
        ca_dn.push(DnType::CommonName, "OmniPort CA");
        ca_params.distinguished_name = ca_dn;
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);

        let ca_key = KeyPair::generate().unwrap();
        let ca_cert = ca_params.self_signed(&ca_key).unwrap();

        // 2. Generate Server Cert signed by CA
        let mut srv_params = CertificateParams::new(vec!["localhost".into()]).unwrap();
        let mut srv_dn = DistinguishedName::new();
        srv_dn.push(DnType::CommonName, "localhost");
        srv_params.distinguished_name = srv_dn;
        let srv_key = KeyPair::generate().unwrap();
        let srv_cert = srv_params.signed_by(&srv_key, &ca_cert, &ca_key).unwrap();

        // 3. Generate Client Cert signed by CA
        let mut cli_params = CertificateParams::default();
        let mut cli_dn = DistinguishedName::new();
        cli_dn.push(DnType::CommonName, "OmniPortClient");
        cli_params.distinguished_name = cli_dn;
        let cli_key = KeyPair::generate().unwrap();
        let cli_cert = cli_params.signed_by(&cli_key, &ca_cert, &ca_key).unwrap();

        let ca_cert_der = CertificateDer::from(ca_cert.der().to_vec());
        let srv_cert_der = vec![CertificateDer::from(srv_cert.der().to_vec())];
        let srv_key_der = PrivateKeyDer::Pkcs8(srv_key.serialize_der().into());
        let cli_cert_der = vec![CertificateDer::from(cli_cert.der().to_vec())];
        let cli_key_der = PrivateKeyDer::Pkcs8(cli_key.serialize_der().into());

        (
            ca_cert_der,
            srv_cert_der,
            srv_key_der,
            cli_cert_der,
            cli_key_der,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::test_helpers::generate_test_mtls_certs;
    use super::*;

    #[tokio::test]
    async fn test_mtls_loopback_handshake_and_io() {
        let (ca, srv_cert, srv_key, cli_cert, cli_key) = generate_test_mtls_certs();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let ca_srv = ca.clone();
        let server_task = tokio::spawn(async move {
            TlsEndpoint::accept_server(
                50,
                "TlsServer",
                &listener,
                ca_srv,
                srv_cert,
                srv_key,
                SerialConfig::default(),
            )
            .await
            .unwrap()
        });

        let server_name = ServerName::try_from("localhost").unwrap();
        let mut client = TlsEndpoint::connect_client(
            51,
            "TlsClient",
            addr,
            server_name,
            ca,
            cli_cert,
            cli_key,
            SerialConfig::default(),
        )
        .await
        .unwrap();

        let mut server = server_task.await.unwrap();

        // Client -> Server
        let send_evt = SerialEvent {
            timestamp_ns: 9999,
            source_id: 51,
            flags: FrameFlags::empty(),
            pins: PinState::empty(),
            payload: Bytes::copy_from_slice(b"SECURE_MTLS_PAYLOAD"),
        };
        client.send(send_evt).await.unwrap();

        let recv_evt = server.recv().await.unwrap();
        assert_eq!(&recv_evt.payload[..], b"SECURE_MTLS_PAYLOAD");
    }
}
