use crate::daemon::DaemonEngine;
use omniport_proto::codec::{read_message, write_message};
use omniport_proto::{ProtoError, Request, Response};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

/// Runs the omniportd control-plane RPC server
pub async fn run_server(
    addr: &str,
    engine: Arc<DaemonEngine>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind(addr).await?;
    let mut shutdown_rx = engine.subscribe_shutdown();
    let mut connections = JoinSet::new();

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

                while connections.try_join_next().is_some() {}

                let eng = Arc::clone(&engine);
                connections.spawn(async move {
                    let mut conn_shutdown_rx = eng.subscribe_shutdown();
                    loop {
                        if eng.is_shutting_down.load(Ordering::SeqCst) {
                            break;
                        }
                        let req: Request = tokio::select! {
                            biased;
                            _ = conn_shutdown_rx.recv() => break,
                            msg = read_message(&mut socket) => match msg {
                                Ok(msg) => msg,
                                Err(ProtoError::ConnectionClosed) => break,
                                Err(e) => {
                                    eprintln!("RPC read error from {}: {}", peer_addr, e);
                                    break;
                                }
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

    drop(listener);
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    connections.abort_all();

    Ok(())
}
