//! Process-level lifecycle tests for the `omniportd` binary.

use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Runs a command, killing it (and failing the test) if it does not exit within
/// `timeout`. This keeps a "daemon unexpectedly starts and runs forever"
/// regression from hanging the whole test binary.
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Option<Output> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn omniportd");
    let start = Instant::now();
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => return Some(child.wait_with_output().expect("wait_with_output")),
            None if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}

/// A daemon that cannot bind its control-plane socket must exit non-zero: a
/// supervisor/CI job must be able to detect the startup failure.
#[test]
fn test_daemon_exits_nonzero_when_bind_address_is_unavailable() {
    // Hold the port so the daemon's bind attempt fails.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let addr = listener.local_addr().expect("local addr");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_omniportd"));
    cmd.args(["--bind", &addr.to_string()]);
    let output = run_with_timeout(cmd, Duration::from_secs(10))
        .expect("omniportd did not exit: it apparently bound an already-used address");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "omniportd must exit non-zero when the bind fails; status={:?} stdout={stdout:?} stderr={stderr:?}",
        output.status
    );
    assert!(
        stderr.contains("failed to start"),
        "startup failure must be reported on stderr; stderr={stderr:?}"
    );
    drop(listener);
}

/// Unknown arguments must be rejected instead of silently falling back to the
/// default bind address (which previously started a daemon on 127.0.0.1:50051).
#[test]
fn test_daemon_rejects_unknown_arguments() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_omniportd"));
    cmd.arg("--definitely-not-a-real-flag");
    let output = run_with_timeout(cmd, Duration::from_secs(10))
        .expect("omniportd did not exit after being passed an unknown flag");

    assert_eq!(
        output.status.code(),
        Some(2),
        "argument errors must exit with code 2; status={:?}",
        output.status
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unrecognized argument"),
        "stderr should explain the argument error; stderr={stderr:?}"
    );
}
