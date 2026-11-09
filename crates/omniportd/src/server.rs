use crate::daemon::DaemonEngine;
use omniport_proto::codec::{read_message, write_message};
use omniport_proto::{ProtoError, Request, Response};
use std::sync::Arc;
use tokio::net::TcpListener;

/// Runs the omniportd control-plane RPC server
pub async fn run_server(
    addr: &str,
    engine: Arc<DaemonEngine>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind(addr).await?;
    let mut shutdown_rx = engine.subscribe_shutdown();

    loop {
        tokio::select! {
            res = listener.accept() => {
                let (mut socket, peer_addr) = match res {
                    Ok(pair) => pair,
                    Err(e) => {
                        eprintln!("omniportd accept error: {}", e);
                        continue;
                    }
                };

                let eng = Arc::clone(&engine);
                tokio::spawn(async move {
                    loop {
                        let req: Request = match read_message(&mut socket).await {
                            Ok(msg) => msg,
                            Err(ProtoError::ConnectionClosed) => break,
                            Err(e) => {
                                eprintln!("RPC read error from {}: {}", peer_addr, e);
                                break;
                            }
                        };

                        let resp: Response = eng.handle_request(req).await;

                        if let Err(e) = write_message(&mut socket, &resp).await {
                            eprintln!("RPC write error to {}: {}", peer_addr, e);
                            break;
                        }
                    }
                });
            }
            _ = shutdown_rx.recv() => {
                break;
            }
        }
    }

    Ok(())
}
