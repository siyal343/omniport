use omniportd::{DaemonEngine, run_server};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().collect();
    let mut bind_addr = "127.0.0.1:50051".to_string();

    let mut i = 1;
    while i < args.len() {
        if (args[i] == "--bind" || args[i] == "-b") && i + 1 < args.len() {
            bind_addr = args[i + 1].clone();
            i += 1;
        }
        i += 1;
    }

    println!(
        "omniportd v0.1.0 starting control plane on {}...",
        bind_addr
    );
    let engine = Arc::new(DaemonEngine::new());
    let engine_clone = Arc::clone(&engine);

    let server_task = tokio::spawn(async move {
        if let Err(e) = run_server(&bind_addr, engine_clone).await {
            eprintln!("omniportd server encountered fatal error: {}", e);
        }
    });

    // Wait for Ctrl+C or internal shutdown signal
    let mut shutdown_rx = engine.subscribe_shutdown();
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\nomniportd received SIGINT/Ctrl+C, initiating shutdown...");
            let _ = engine.shutdown_tx.send(());
        }
        _ = shutdown_rx.recv() => {
            println!("\nomniportd received remote shutdown RPC request...");
        }
        _ = server_task => {}
    }

    println!("omniportd exited cleanly.");
    Ok(())
}
