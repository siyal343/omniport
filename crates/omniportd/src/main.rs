use omniportd::{DaemonEngine, ServerConfig, run_server_with_config};
use std::sync::Arc;
use std::time::Duration;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const USAGE: &str = "Usage: omniportd [--bind <ADDR>] [--allow-remote]
                    [--max-connections <N>] [--idle-timeout-secs <N>]
                    [--request-timeout-secs <N>] [--max-requests-per-sec <N>]";

/// Parsed command line: the server policy plus the explicit remote-bind opt-in.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CliOptions {
    config: ServerConfig,
    allow_remote: bool,
}

fn parse_value<T: std::str::FromStr>(
    args: &[String],
    flag: &str,
    index: &mut usize,
) -> Result<T, String> {
    let value = args
        .get(*index + 1)
        .ok_or_else(|| format!("{flag} requires a value"))?;
    let parsed = value
        .parse::<T>()
        .map_err(|_| format!("invalid value for {flag}: {value}"))?;
    *index += 2;
    Ok(parsed)
}

/// Parses the daemon command line.
///
/// Unknown arguments are a hard error rather than being silently ignored (a typo
/// used to fall back to the default address), and a non-loopback `--bind`
/// requires `--allow-remote` because the control plane has no authentication.
fn parse_args(args: &[String]) -> Result<CliOptions, String> {
    let mut config = ServerConfig::default();
    let mut allow_remote = false;
    let mut i = 1;

    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--bind" | "-b" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| format!("{arg} requires an address value"))?;
                config.bind_addr = value.clone();
                i += 2;
            }
            "--allow-remote" => {
                allow_remote = true;
                i += 1;
            }
            "--max-connections" => {
                config.max_connections = parse_value(args, arg, &mut i)?;
            }
            "--idle-timeout-secs" => {
                let secs: u64 = parse_value(args, arg, &mut i)?;
                config.idle_timeout = Duration::from_secs(secs);
            }
            "--request-timeout-secs" => {
                let secs: u64 = parse_value(args, arg, &mut i)?;
                config.request_timeout = Duration::from_secs(secs);
            }
            "--max-requests-per-sec" => {
                config.max_requests_per_second = parse_value(args, arg, &mut i)?;
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other if other.starts_with("--bind=") => {
                let value = other.trim_start_matches("--bind=");
                if value.is_empty() {
                    return Err("--bind requires an address value".to_string());
                }
                config.bind_addr = value.to_string();
                i += 1;
            }
            other => return Err(format!("unrecognized argument: {other}")),
        }
    }

    config.validate(allow_remote)?;
    Ok(CliOptions {
        config,
        allow_remote,
    })
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let args: Vec<String> = std::env::args().collect();
    let options = match parse_args(&args) {
        Ok(options) => options,
        Err(err) => {
            eprintln!("omniportd: {err}");
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    let config = options.config;
    let bind_addr = config.bind_addr.clone();

    println!(
        "omniportd v0.1.0 starting control plane on {} (max_connections={}, idle_timeout={:?}, request_timeout={:?}, rate_limit={}/s{})...",
        bind_addr,
        config.max_connections,
        config.idle_timeout,
        config.request_timeout,
        config.max_requests_per_second,
        if options.allow_remote {
            ", remote control enabled"
        } else {
            ""
        }
    );
    let engine = Arc::new(DaemonEngine::new());
    let engine_clone = Arc::clone(&engine);

    let server_task =
        tokio::spawn(async move { run_server_with_config(config, engine_clone).await });

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
    use super::{CliOptions, parse_args};
    use omniportd::ServerConfig;
    use std::time::Duration;

    fn args(list: &[&str]) -> Vec<String> {
        std::iter::once("omniportd".to_string())
            .chain(list.iter().map(|s| s.to_string()))
            .collect()
    }

    #[test]
    fn test_parse_args_defaults() {
        let parsed = parse_args(&args(&[])).unwrap();
        assert_eq!(parsed.config, ServerConfig::default());
        assert!(!parsed.allow_remote);
    }

    #[test]
    fn test_parse_args_bind_forms() {
        assert_eq!(
            parse_args(&args(&["--bind", "127.0.0.1:1234"]))
                .unwrap()
                .config
                .bind_addr,
            "127.0.0.1:1234"
        );
        assert_eq!(
            parse_args(&args(&["-b", "127.0.0.1:9"]))
                .unwrap()
                .config
                .bind_addr,
            "127.0.0.1:9"
        );
        assert_eq!(
            parse_args(&args(&["--bind=127.0.0.1:7777"]))
                .unwrap()
                .config
                .bind_addr,
            "127.0.0.1:7777"
        );
    }

    #[test]
    fn test_parse_args_limits_and_remote_opt_in() {
        let parsed = parse_args(&args(&[
            "--bind",
            "0.0.0.0:1234",
            "--allow-remote",
            "--max-connections",
            "8",
            "--idle-timeout-secs",
            "5",
            "--request-timeout-secs",
            "2",
            "--max-requests-per-sec",
            "100",
        ]))
        .unwrap();
        assert!(parsed.allow_remote);
        assert_eq!(parsed.config.bind_addr, "0.0.0.0:1234");
        assert_eq!(parsed.config.max_connections, 8);
        assert_eq!(parsed.config.idle_timeout, Duration::from_secs(5));
        assert_eq!(parsed.config.request_timeout, Duration::from_secs(2));
        assert_eq!(parsed.config.max_requests_per_second, 100);
    }

    #[test]
    fn test_parse_args_rejects_remote_bind_without_opt_in() {
        let err = parse_args(&args(&["--bind", "0.0.0.0:1234"])).unwrap_err();
        assert!(err.contains("--allow-remote"), "unexpected error: {err}");
        // Loopback is always allowed without the flag.
        assert!(parse_args(&args(&["--bind", "127.0.0.1:1234"])).is_ok());
    }

    #[test]
    fn test_parse_args_rejects_missing_or_unknown_values() {
        assert!(parse_args(&args(&["--bind"])).is_err());
        assert!(parse_args(&args(&["--bind="])).is_err());
        assert!(parse_args(&args(&["--bogus"])).is_err());
        assert!(parse_args(&args(&["--max-connections"])).is_err());
        assert!(parse_args(&args(&["--max-connections", "zero"])).is_err());
        assert!(parse_args(&args(&["--max-connections", "0"])).is_err());
        assert!(parse_args(&args(&["--idle-timeout-secs", "0"])).is_err());
    }

    #[test]
    fn test_cli_options_are_comparable() {
        // Guards accidental derive removal (tests compare parsed options).
        let parsed = parse_args(&args(&[])).unwrap();
        let expected = CliOptions {
            config: ServerConfig::default(),
            allow_remote: false,
        };
        assert_eq!(parsed, expected);
    }
}
