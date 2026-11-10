use omniportd::{DaemonEngine, run_server};
use std::sync::Arc;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const USAGE: &str = "Usage: omniportd [--bind <ADDR>]";

/// Parses the daemon command line. Supports `--bind ADDR`, `-b ADDR` and
/// `--bind=ADDR`; unknown arguments are a hard error rather than being
/// silently ignored (a typo used to fall back to the default address).
fn parse_args(args: &[String]) -> Result<String, String> {
    let mut bind_addr = "127.0.0.1:50051".to_string();
    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--bind" || arg == "-b" {
            let value = args
                .get(i + 1)
                .ok_or_else(|| format!("{arg} requires an address value"))?;
            bind_addr = value.clone();
            i += 2;
        } else if let Some(value) = arg.strip_prefix("--bind=") {
            if value.is_empty() {
                return Err("--bind requires an address value".to_string());
            }
            bind_addr = value.to_string();
            i += 1;
        } else if arg == "--help" || arg == "-h" {
            println!("{USAGE}");
            std::process::exit(0);
        } else {
            return Err(format!("unrecognized argument: {arg}"));
        }
    }
    Ok(bind_addr)
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let args: Vec<String> = std::env::args().collect();
    let bind_addr = match parse_args(&args) {
        Ok(addr) => addr,
        Err(err) => {
            eprintln!("omniportd: {err}");
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };

    println!(
        "omniportd v0.1.0 starting control plane on {}...",
        bind_addr
    );
    let engine = Arc::new(DaemonEngine::new());
    let engine_clone = Arc::clone(&engine);

    let server_task = tokio::spawn(async move { run_server(&bind_addr, engine_clone).await });

    // Wait for Ctrl+C or internal shutdown signal
    let mut shutdown_rx = engine.subscribe_shutdown();
    let outcome: Result<(), BoxError> = tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\nomniportd received SIGINT/Ctrl+C, initiating shutdown...");
            let _ = engine.shutdown_tx.send(());
            Ok(())
        }
        _ = shutdown_rx.recv() => {
            println!("\nomniportd received remote shutdown RPC request...");
            Ok(())
        }
        res = server_task => {
            // A fatal server error (e.g. the bind address is already in use)
            // must propagate so the process exits non-zero.
            match res {
                Ok(inner) => inner,
                Err(join_err) => Err(Box::new(join_err) as BoxError),
            }
        }
    };
    outcome.map_err(|e| format!("omniportd failed to start: {e}"))?;

    println!("omniportd exited cleanly.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_args;

    fn args(list: &[&str]) -> Vec<String> {
        std::iter::once("omniportd".to_string())
            .chain(list.iter().map(|s| s.to_string()))
            .collect()
    }

    #[test]
    fn test_parse_args_bind_forms() {
        assert_eq!(parse_args(&args(&[])).unwrap(), "127.0.0.1:50051");
        assert_eq!(
            parse_args(&args(&["--bind", "0.0.0.0:1234"])).unwrap(),
            "0.0.0.0:1234"
        );
        assert_eq!(
            parse_args(&args(&["-b", "127.0.0.1:9"])).unwrap(),
            "127.0.0.1:9"
        );
        assert_eq!(
            parse_args(&args(&["--bind=127.0.0.1:7777"])).unwrap(),
            "127.0.0.1:7777"
        );
    }

    #[test]
    fn test_parse_args_rejects_missing_or_unknown_values() {
        assert!(parse_args(&args(&["--bind"])).is_err());
        assert!(parse_args(&args(&["--bind="])).is_err());
        assert!(parse_args(&args(&["--bogus"])).is_err());
    }
}
