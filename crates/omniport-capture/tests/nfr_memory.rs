//! NFR-103 memory harness: the desktop capture store must stay under 250 MB of
//! resident memory while a 10 GiB capture session is mapped.
//!
//! The dual-mmap design maps the whole session but only pages in the rows the
//! GUI asks for. This test creates sparse files (10 GiB data region plus the
//! proportional index), appends a realistic burst of frames, reads several
//! viewport windows, and samples the process working set before/after. Marked
//! `#[ignore]` and executed by the release `perf` CI job (`--ignored`).

use omniport_capture::{FrameIndexEntry, MmapCaptureEngine};

const DATA_BYTES: u64 = 10 * 1024 * 1024 * 1024; // 10 GiB capture session
const AVG_FRAME_BYTES: u64 = 64;
const APPENDED_FRAMES: u64 = 200_000;
const PAYLOAD_BYTES: usize = 64;
const RSS_LIMIT_BYTES: u64 = 250 * 1024 * 1024;

#[cfg(target_os = "linux")]
fn resident_set_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: u64 = rest
                .split_whitespace()
                .next()
                .and_then(|value| value.parse().ok())?;
            return Some(kb * 1024);
        }
    }
    None
}

#[cfg(windows)]
fn resident_set_bytes() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    // SAFETY: passing a zeroed PROCESS_MEMORY_COUNTERS with its `cb` size set to
    // the real struct size, as required by GetProcessMemoryInfo; the handle is
    // the current process pseudo-handle and is not closed.
    unsafe {
        let mut counters: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) != 0 {
            Some(counters.WorkingSetSize as u64)
        } else {
            None
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn resident_set_bytes() -> Option<u64> {
    None
}

fn megabytes(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

#[test]
#[ignore = "performance harness; run by the perf CI job in release mode"]
fn test_nfr103_desktop_memory_over_10gb_capture() {
    let dir = tempfile::tempdir().expect("temporary capture directory");
    let max_packets = DATA_BYTES / AVG_FRAME_BYTES; // index sized for 64-byte frames
    let index_bytes = max_packets * std::mem::size_of::<FrameIndexEntry>() as u64;

    let baseline = resident_set_bytes();
    let mut engine = MmapCaptureEngine::create(dir.path(), max_packets, DATA_BYTES)
        .expect("create sparse 10 GiB dual-mmap capture store");

    // A short burst of real frames writes only the first rows/pages, exactly like
    // a freshly started session that already mapped its full capacity.
    let payload = [0x33_u8; PAYLOAD_BYTES];
    for i in 0..APPENDED_FRAMES {
        engine.append_frame(
            FrameIndexEntry {
                timestamp_ns: i,
                data_offset: 0,
                payload_len: PAYLOAD_BYTES as u32,
                endpoint_id: (i % 4 + 1) as u16,
                flags: 0,
                pins: 0,
            },
            &payload,
        );
    }
    assert_eq!(engine.packet_count(), APPENDED_FRAMES);
    assert_eq!(
        engine.dropped_frames(),
        0,
        "sparse store must accept frames"
    );

    // Read viewport windows across the appended rows (the GUI's normal access).
    let mut wire = Vec::new();
    for start in [0_u64, APPENDED_FRAMES / 2, APPENDED_FRAMES - 200] {
        wire.clear();
        engine.read_viewport(start, 200, &mut wire);
        assert!(!wire.is_empty(), "viewport window at row {start} is empty");
    }

    let after = resident_set_bytes();
    let mapped = megabytes(DATA_BYTES + index_bytes);
    match (baseline, after) {
        (Some(before), Some(current)) => {
            let delta = current.saturating_sub(before);
            println!(
                "NFR-103: mapped {mapped:.0} MiB (10 GiB data + index), appended {APPENDED_FRAMES} frames — \
                 RSS before={:.1} MiB, after={:.1} MiB, delta={:.1} MiB (target < {:.0} MiB)",
                megabytes(before),
                megabytes(current),
                megabytes(delta),
                megabytes(RSS_LIMIT_BYTES)
            );
            assert!(
                current < RSS_LIMIT_BYTES,
                "NFR-103 violated: resident set {:.1} MiB exceeds target {:.1} MiB",
                megabytes(current),
                megabytes(RSS_LIMIT_BYTES)
            );
            assert!(
                delta < RSS_LIMIT_BYTES,
                "NFR-103 violated: mapping a 10 GiB capture grew RSS by {:.1} MiB (limit {:.1} MiB)",
                megabytes(delta),
                megabytes(RSS_LIMIT_BYTES)
            );
        }
        _ => {
            // RSS sampling is only implemented for Windows/Linux CI hosts.
            eprintln!(
                "NFR-103: RSS sampling unavailable on this platform; mapped {mapped:.0} MiB without asserting"
            );
        }
    }
}
