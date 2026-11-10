//! Control-plane hardening tests: idle deadline, concurrency cap, request-rate
//! limit, and oversized-frame handling against a real TCP server.

use omniport_proto::codec::{read_message, write_message};
use omniport_proto::{MAX_FRAME_SIZE, ProtoError, Request, Response, RpcClient};
use omniportd::{DaemonEngine, ServerConfig, run_server_with_config};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

async fn allocate_ephemeral_addr() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    drop(listener);
    addr
}

async fn wait_until_listening(addr: &str) {
    for _ in 0..80 {
        if TcpStream::connect(addr).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("omniportd test server failed to start listening on {addr}");
}

async fn start_server(config: ServerConfig) -> (tokio::task::JoinHandle<()>, Arc<DaemonEngine>) {
    let addr = config.bind_addr.clone();
    let engine = Arc::new(DaemonEngine::new());
    let server_engine = Arc::clone(&engine);
    let handle = tokio::spawn(async move {
        run_server_with_config(config, server_engine).await.unwrap();
    });
    wait_until_listening(&addr).await;
    // The readiness probe itself is an accepted connection that is then dropped;
    // give its handler a moment to release its slot before tests assert on limits.
    tokio::time::sleep(Duration::from_millis(100)).await;
    (handle, engine)
}

async fn shutdown(engine: &Arc<DaemonEngine>, handle: tokio::task::JoinHandle<()>) {
    let _ = engine.shutdown_tx.send(());
    tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("server did not shut down in time")
        .unwrap();
}

#[tokio::test]
async fn test_idle_connection_is_closed_without_unsolicited_frame() {
    let addr = allocate_ephemeral_addr().await;
    let config = ServerConfig {
        bind_addr: addr.clone(),
        idle_timeout: Duration::from_millis(250),
        ..ServerConfig::default()
    };
    let (server, engine) = start_server(config).await;

    let mut socket = TcpStream::connect(&addr).await.unwrap();
    let started = Instant::now();
    let first_read = tokio::time::timeout(
        Duration::from_secs(3),
        read_message::<_, Response>(&mut socket),
    )
    .await
    .expect("server did not close the idle connection");
    assert!(
        matches!(first_read, Err(ProtoError::ConnectionClosed)),
        "idle connections must close silently, not receive a frame: {first_read:?}"
    );
    assert!(
        started.elapsed() >= Duration::from_millis(200),
        "server closed the connection before the idle deadline"
    );

    // The server must still accept new clients after reaping an idle one.
    let mut client = RpcClient::connect(&addr).await.unwrap();
    assert!(client.health_check().await.is_ok());

    shutdown(&engine, server).await;
}

#[tokio::test]
async fn test_connection_limit_rejects_with_explicit_503() {
    let addr = allocate_ephemeral_addr().await;
    let config = ServerConfig {
        bind_addr: addr.clone(),
        max_connections: 1,
        ..ServerConfig::default()
    };
    let (server, engine) = start_server(config).await;

    // The first client occupies the only connection slot.
    let mut first = RpcClient::connect(&addr).await.unwrap();
    assert!(first.health_check().await.is_ok());

    // The second client's request receives an explicit 503 rather than a hang.
    let mut second = TcpStream::connect(&addr).await.unwrap();
    write_message(&mut second, &Request::HealthCheck)
        .await
        .unwrap();
    let response: Response =
        tokio::time::timeout(Duration::from_secs(3), read_message(&mut second))
            .await
            .expect("over-limit connection was not answered")
            .unwrap();
    match response {
        Response::Error { code, message } => {
            assert_eq!(code, 503);
            assert!(
                message.contains("connection limit"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected 503 connection-limit error, got {other:?}"),
    }

    // The admitted connection keeps working.
    assert!(
        first.health_check().await.is_ok(),
        "first client lost its slot"
    );

    shutdown(&engine, server).await;
}

#[tokio::test]
async fn test_request_rate_limit_returns_429_then_closes() {
    let addr = allocate_ephemeral_addr().await;
    let config = ServerConfig {
        bind_addr: addr.clone(),
        max_requests_per_second: 1,
        ..ServerConfig::default()
    };
    let (server, engine) = start_server(config).await;

    let mut client = RpcClient::connect(&addr).await.unwrap();
    assert!(
        client.health_check().await.is_ok(),
        "first request is within the limit"
    );
    let second = client.health_check().await;
    match second {
        Err(ProtoError::ServerError(message)) => assert!(
            message.contains("rate limit"),
            "unexpected server error: {message}"
        ),
        other => panic!("expected a 429 rate-limit error, got {other:?}"),
    }

    shutdown(&engine, server).await;
}

#[tokio::test]
async fn test_oversized_frame_closes_connection() {
    let addr = allocate_ephemeral_addr().await;
    let config = ServerConfig {
        bind_addr: addr.clone(),
        ..ServerConfig::default()
    };
    let (server, engine) = start_server(config).await;

    let mut socket = TcpStream::connect(&addr).await.unwrap();
    let oversized = (MAX_FRAME_SIZE as u32) + 1;
    socket.write_all(&oversized.to_be_bytes()).await.unwrap();
    socket.flush().await.unwrap();

    let result = tokio::time::timeout(
        Duration::from_secs(3),
        read_message::<_, Response>(&mut socket),
    )
    .await
    .expect("server did not close an oversized-frame connection");
    assert!(
        matches!(result, Err(ProtoError::ConnectionClosed)),
        "expected a closed connection, got {result:?}"
    );

    // The server keeps serving other clients.
    let mut client = RpcClient::connect(&addr).await.unwrap();
    assert!(client.health_check().await.is_ok());

    shutdown(&engine, server).await;
}

#[tokio::test]
async fn test_shutdown_wakes_idle_connections() {
    let addr = allocate_ephemeral_addr().await;
    let config = ServerConfig {
        bind_addr: addr.clone(),
        idle_timeout: Duration::from_secs(30),
        ..ServerConfig::default()
    };
    let (server, engine) = start_server(config).await;

    // Hold an idle open connection, then shut down; the server must not wait for
    // the (long) idle deadline to drain.
    let _idle = TcpStream::connect(&addr).await.unwrap();
    let started = Instant::now();
    shutdown(&engine, server).await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "shutdown waited for the idle deadline"
    );
}
