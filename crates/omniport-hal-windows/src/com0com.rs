use crate::error::{HalError, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

static NEXT_PAIR_ID: AtomicU32 = AtomicU32::new(100);

/// Metadata representing an active virtual serial port pair
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualPair {
    pub id: u32,
    pub port_a: String,
    pub port_b: String,
    pub is_hardware: bool,
}

/// RAII Guard that removes the virtual COM pair upon destruction
#[derive(Debug)]
pub struct VirtualPairGuard {
    pair: VirtualPair,
    controller: Arc<Mutex<Com0ComControllerInner>>,
    active: bool,
}

impl VirtualPairGuard {
    pub fn pair(&self) -> &VirtualPair {
        &self.pair
    }

    pub fn port_a(&self) -> &str {
        &self.pair.port_a
    }

    pub fn port_b(&self) -> &str {
        &self.pair.port_b
    }

    pub fn remove(mut self) -> Result<()> {
        let res = self.cleanup();
        // Do not retry cleanup in Drop after an explicit remove() call.
        self.active = false;
        res
    }

    fn cleanup(&mut self) -> Result<()> {
        if self.active {
            let mut inner = self
                .controller
                .lock()
                .map_err(|_| HalError::Com0Com("controller lock poisoned".into()))?;
            inner.remove_pair(self.pair.id)?;
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for VirtualPairGuard {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[derive(Debug)]
struct Com0ComControllerInner {
    pairs: HashMap<u32, VirtualPair>,
    setupc_path: Option<PathBuf>,
    mock_mode: bool,
}

impl Com0ComControllerInner {
    fn new() -> Self {
        let setupc_path = find_setupc_binary();
        Self {
            pairs: HashMap::new(),
            setupc_path,
            mock_mode: false,
        }
    }

    fn new_mock() -> Self {
        Self {
            pairs: HashMap::new(),
            setupc_path: None,
            mock_mode: true,
        }
    }

    #[cfg(test)]
    fn with_setupc_path(setupc_path: Option<PathBuf>) -> Self {
        Self {
            pairs: HashMap::new(),
            setupc_path,
            mock_mode: false,
        }
    }

    fn is_driver_available(&self) -> bool {
        if self.setupc_path.is_some() {
            return true;
        }
        #[cfg(windows)]
        {
            // Check if \\.\com0com control device is accessible
            use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
            use windows_sys::Win32::Storage::FileSystem::{
                CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING,
            };

            let path: Vec<u16> = r"\\.\com0com"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let handle = unsafe {
                CreateFileW(
                    path.as_ptr(),
                    0,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    std::ptr::null_mut(),
                )
            };
            if handle != INVALID_HANDLE_VALUE && !handle.is_null() {
                unsafe { CloseHandle(handle) };
                return true;
            }
        }
        false
    }

    fn create_pair(&mut self, port_a: &str, port_b: &str) -> Result<VirtualPair> {
        let port_a = port_a.trim();
        let port_b = port_b.trim();
        if port_a.is_empty() || port_b.is_empty() {
            return Err(HalError::InvalidParameter(
                "virtual pair port names must not be empty".into(),
            ));
        }
        if port_a.eq_ignore_ascii_case(port_b) {
            return Err(HalError::InvalidParameter(format!(
                "virtual pair endpoints must differ (both '{port_a}')"
            )));
        }
        for existing in self.pairs.values() {
            if existing.port_a.eq_ignore_ascii_case(port_a)
                || existing.port_b.eq_ignore_ascii_case(port_a)
                || existing.port_a.eq_ignore_ascii_case(port_b)
                || existing.port_b.eq_ignore_ascii_case(port_b)
            {
                return Err(HalError::InvalidParameter(format!(
                    "port name already managed by pair {}: ({}, {})",
                    existing.id, existing.port_a, existing.port_b
                )));
            }
        }

        let (pair_id, is_hardware) = if self.mock_mode {
            (NEXT_PAIR_ID.fetch_add(1, Ordering::SeqCst), false)
        } else {
            let setupc = self.setupc_path.as_ref().ok_or_else(|| {
                HalError::Com0ComDriverNotFound(
                    "com0com setupc.exe utility not found in standard paths; cannot create hardware virtual COM pair".into(),
                )
            })?;

            // Invoke setupc.exe install PortName=port_a PortName=port_b
            let output = std::process::Command::new(setupc)
                .args([
                    "install",
                    &format!("PortName={port_a}"),
                    &format!("PortName={port_b}"),
                ])
                .output()
                .map_err(|e| HalError::Com0Com(format!("failed to execute setupc.exe: {e}")))?;

            if !output.status.success() {
                let err_msg = String::from_utf8_lossy(&output.stderr);
                let out_msg = String::from_utf8_lossy(&output.stdout);
                return Err(HalError::Com0Com(format!(
                    "setupc.exe install failed: {} {}",
                    err_msg.trim(),
                    out_msg.trim()
                )));
            }

            let install_stdout = String::from_utf8_lossy(&output.stdout);
            let pair_index = match parse_pair_index_from_install(&install_stdout, port_a, port_b) {
                Some(idx) => idx,
                None => {
                    let list_output = std::process::Command::new(setupc)
                        .arg("list")
                        .output()
                        .map_err(|e| {
                            HalError::Com0Com(format!("failed to execute setupc.exe list: {e}"))
                        })?;
                    if !list_output.status.success() {
                        let err_msg = String::from_utf8_lossy(&list_output.stderr);
                        return Err(HalError::Com0Com(format!(
                            "setupc.exe list failed while resolving pair index: {}",
                            err_msg.trim()
                        )));
                    }
                    let list_stdout = String::from_utf8_lossy(&list_output.stdout);
                    parse_pair_index_from_list(&list_stdout, port_a, port_b).ok_or_else(|| {
                        HalError::Com0Com(format!(
                            "unable to resolve com0com pair index for ({port_a}, {port_b}) from setupc output"
                        ))
                    })?
                }
            };
            (pair_index, true)
        };

        let pair = VirtualPair {
            id: pair_id,
            port_a: port_a.to_string(),
            port_b: port_b.to_string(),
            is_hardware,
        };

        self.pairs.insert(pair_id, pair.clone());
        Ok(pair)
    }

    fn remove_pair(&mut self, id: u32) -> Result<()> {
        let Some(pair) = self.pairs.get(&id).cloned() else {
            return Ok(());
        };
        if pair.is_hardware {
            let setupc = self.setupc_path.as_ref().ok_or_else(|| {
                HalError::Com0ComDriverNotFound(
                    "com0com setupc.exe unavailable while removing hardware virtual pair".into(),
                )
            })?;
            let output = std::process::Command::new(setupc)
                .args(["remove", &id.to_string()])
                .output()
                .map_err(|e| {
                    HalError::Com0Com(format!("failed to execute setupc.exe remove {id}: {e}"))
                })?;
            if !output.status.success() {
                let err_msg = String::from_utf8_lossy(&output.stderr);
                let out_msg = String::from_utf8_lossy(&output.stdout);
                return Err(HalError::Com0Com(format!(
                    "setupc.exe remove {id} failed: {} {}",
                    err_msg.trim(),
                    out_msg.trim()
                )));
            }
        }
        self.pairs.remove(&id);
        Ok(())
    }

    fn list_pairs(&self) -> Vec<VirtualPair> {
        self.pairs.values().cloned().collect()
    }
}

/// Programmatic virtual pair controller wrapping com0com / SetupAPI
#[derive(Debug, Clone)]
pub struct Com0ComController {
    inner: Arc<Mutex<Com0ComControllerInner>>,
}

impl Default for Com0ComController {
    fn default() -> Self {
        Self::new()
    }
}

impl Com0ComController {
    /// Creates a hardware-backed `com0com` controller. Pair creation fails with
    /// `HalError::Com0ComDriverNotFound` unless `setupc.exe` is installed.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Com0ComControllerInner::new())),
        }
    }

    /// Creates an explicit in-memory mock controller for tests on machines without
    /// the kernel driver installed.
    pub fn new_mock() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Com0ComControllerInner::new_mock())),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_setupc_path_for_test(setupc_path: Option<PathBuf>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Com0ComControllerInner::with_setupc_path(
                setupc_path,
            ))),
        }
    }

    pub fn is_driver_available(&self) -> bool {
        self.inner
            .lock()
            .map(|i| i.is_driver_available())
            .unwrap_or(false)
    }

    /// Creates a virtual COM pair. Returns a `VirtualPairGuard` that automatically
    /// removes the pair when dropped.
    pub fn create_pair(&self, port_a: &str, port_b: &str) -> Result<VirtualPairGuard> {
        let pair = {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| HalError::Com0Com("controller lock poisoned".into()))?;
            inner.create_pair(port_a, port_b)?
        };

        Ok(VirtualPairGuard {
            pair,
            controller: Arc::clone(&self.inner),
            active: true,
        })
    }

    pub fn list_pairs(&self) -> Result<Vec<VirtualPair>> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| HalError::Com0Com("controller lock poisoned".into()))?;
        Ok(inner.list_pairs())
    }

    /// Finds available COM port names not currently registered in the active pairs
    pub fn find_free_pair_names(&self, base_start: u32) -> Result<(String, String)> {
        let active = self.list_pairs()?;
        let used_names: std::collections::HashSet<String> = active
            .into_iter()
            .flat_map(|p| vec![p.port_a, p.port_b])
            .collect();

        let mut candidate = base_start;
        loop {
            let name_a = format!("COM{}", candidate);
            let name_b = format!("COM{}", candidate + 1);
            if !used_names.contains(&name_a) && !used_names.contains(&name_b) {
                return Ok((name_a, name_b));
            }
            candidate += 2;
        }
    }
}

fn parse_cnc_line(line: &str) -> Option<(u32, Option<&str>)> {
    let trimmed = line.trim();
    let mut parts = trimmed.split_whitespace();
    let head = parts.next()?;
    let upper = head.to_ascii_uppercase();
    let idx_str = upper
        .strip_prefix("CNCA")
        .or_else(|| upper.strip_prefix("CNCB"))?;
    let index: u32 = idx_str.parse().ok()?;

    let mut port_name = None;
    for token in parts {
        for kv in token.split(',') {
            if let Some((k, v)) = kv.split_once('=')
                && k.eq_ignore_ascii_case("PortName")
                && !v.is_empty()
            {
                port_name = Some(v);
            }
        }
    }
    Some((index, port_name))
}

/// Resolves the `com0com` virtual bus pair index matching `(port_a, port_b)` from `setupc list` output.
pub fn parse_pair_index_from_list(output: &str, port_a: &str, port_b: &str) -> Option<u32> {
    let mut by_index: HashMap<u32, Vec<&str>> = HashMap::new();
    for line in output.lines() {
        if let Some((idx, Some(port))) = parse_cnc_line(line) {
            by_index.entry(idx).or_default().push(port);
        }
    }

    by_index.into_iter().find_map(|(idx, ports)| {
        let has_a = ports.iter().any(|p| p.eq_ignore_ascii_case(port_a));
        let has_b = ports.iter().any(|p| p.eq_ignore_ascii_case(port_b));
        if has_a && has_b { Some(idx) } else { None }
    })
}

/// Resolves the newly installed `com0com` pair index from `setupc install` stdout.
pub fn parse_pair_index_from_install(output: &str, port_a: &str, port_b: &str) -> Option<u32> {
    if let Some(idx) = parse_pair_index_from_list(output, port_a, port_b) {
        return Some(idx);
    }

    // Some setupc builds emit bare `CNCA<N>` / `CNCB<N>` lines on `install`
    // without echoing `PortName=...`; accept when exactly one index appears.
    let mut unique: Option<u32> = None;
    for line in output.lines() {
        if let Some((idx, _)) = parse_cnc_line(line) {
            match unique {
                None => unique = Some(idx),
                Some(prev) if prev == idx => {}
                Some(_) => return None,
            }
        }
    }
    unique
}

fn find_setupc_binary() -> Option<PathBuf> {
    let standard_paths = [
        r"C:\Program Files (x86)\com0com\setupc.exe",
        r"C:\Program Files\com0com\setupc.exe",
    ];

    for path_str in standard_paths {
        let path = Path::new(path_str);
        if path.exists() {
            return Some(path.to_path_buf());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_virtual_pair_lifecycle_and_raii() {
        let controller = Com0ComController::new_mock();

        let (port_a, port_b) = controller.find_free_pair_names(20).unwrap();
        assert_eq!(port_a, "COM20");
        assert_eq!(port_b, "COM21");

        {
            let guard = controller.create_pair(&port_a, &port_b).unwrap();
            assert_eq!(guard.port_a(), "COM20");
            assert_eq!(guard.port_b(), "COM21");
            assert!(!guard.pair().is_hardware);

            let pairs = controller.list_pairs().unwrap();
            assert_eq!(pairs.len(), 1);
            assert_eq!(pairs[0].port_a, "COM20");
            assert_eq!(pairs[0].port_b, "COM21");
        }

        // Guard dropped: pair should be removed
        let pairs_after = controller.list_pairs().unwrap();
        assert_eq!(pairs_after.len(), 0);
    }

    #[test]
    fn test_virtual_pair_explicit_remove() {
        let controller = Com0ComController::new_mock();

        let guard = controller.create_pair("COM30", "COM31").unwrap();
        assert_eq!(controller.list_pairs().unwrap().len(), 1);

        guard.remove().unwrap();
        assert_eq!(controller.list_pairs().unwrap().len(), 0);
    }

    #[test]
    fn test_create_pair_fails_loudly_when_driver_absent() {
        let controller = Com0ComController::with_setupc_path_for_test(None);
        let err = controller
            .create_pair("COM40", "COM41")
            .expect_err("create_pair without setupc/driver must fail loudly");
        assert!(
            matches!(err, HalError::Com0ComDriverNotFound(_)),
            "expected Com0ComDriverNotFound, got {err:?}"
        );
    }

    #[test]
    fn test_create_pair_rejects_invalid_or_duplicate_port_names() {
        let controller = Com0ComController::new_mock();
        assert!(matches!(
            controller.create_pair("", "COM41"),
            Err(HalError::InvalidParameter(_))
        ));
        assert!(matches!(
            controller.create_pair("COM40", "com40"),
            Err(HalError::InvalidParameter(_))
        ));

        let _guard = controller.create_pair("COM40", "COM41").unwrap();
        assert!(matches!(
            controller.create_pair("COM41", "COM42"),
            Err(HalError::InvalidParameter(_))
        ));
    }

    #[test]
    fn test_parse_pair_index_from_setupc_output() {
        let list_out = "\
       CNCA0 PortName=COM8,EmuBR=yes\n\
       CNCB0 PortName=COM9,EmuBR=yes\n\
       CNCA7 PortName=COM24,EmuOverrun=no\n\
       CNCB7 PortName=COM25\n";
        assert_eq!(
            parse_pair_index_from_list(list_out, "com24", "COM25"),
            Some(7)
        );
        assert_eq!(parse_pair_index_from_list(list_out, "COM8", "COM25"), None);

        let install_bare = "       CNCA3\n       CNCB3\n";
        assert_eq!(
            parse_pair_index_from_install(install_bare, "COM50", "COM51"),
            Some(3)
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_hardware_controller_uses_real_pair_index_for_install_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("setupc_stub.cmd");
        let log = dir.path().join("calls.log");
        std::fs::write(
            &script,
            format!(
                "@echo off\r\n\
                 echo [%~1:%~2:%~3]>>\"{log}\"\r\n\
                 if \"%~1\"==\"install\" (\r\n\
                   echo        CNCA7 PortName=COM60,EmuBR=yes\r\n\
                   echo        CNCB7 PortName=COM61,EmuBR=yes\r\n\
                   exit /b 0\r\n\
                 )\r\n\
                 if \"%~1\"==\"remove\" (\r\n\
                   if \"%~2\"==\"7\" exit /b 0\r\n\
                   exit /b 1\r\n\
                 )\r\n\
                 exit /b 2\r\n",
                log = log.display()
            ),
        )
        .unwrap();

        let controller = Com0ComController::with_setupc_path_for_test(Some(script.clone()));
        let guard = controller.create_pair("COM60", "COM61").unwrap();
        assert_eq!(guard.pair().id, 7);
        assert!(guard.pair().is_hardware);

        guard.remove().unwrap();

        let recorded = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = recorded.lines().map(str::trim).collect();
        assert_eq!(
            lines,
            vec!["[install:PortName=COM60:PortName=COM61]", "[remove:7:]"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_hardware_controller_falls_back_to_list_and_surfaces_remove_error() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("setupc_list_stub.cmd");
        std::fs::write(
            &script,
            "@echo off\r\n\
             if \"%1\"==\"install\" (\r\n\
               echo ok\r\n\
               exit /b 0\r\n\
             )\r\n\
             if \"%1\"==\"list\" (\r\n\
               echo        CNCA12 PortName=COM70,EmuBR=yes\r\n\
               echo        CNCB12 PortName=COM71\r\n\
               exit /b 0\r\n\
             )\r\n\
             if \"%1\"==\"remove\" (\r\n\
               >&2 echo remove denied\r\n\
               exit /b 5\r\n\
             )\r\n\
             exit /b 1\r\n",
        )
        .unwrap();

        let controller = Com0ComController::with_setupc_path_for_test(Some(script));
        let guard = controller.create_pair("COM70", "COM71").unwrap();
        assert_eq!(guard.pair().id, 12);

        let err = guard
            .remove()
            .expect_err("explicit remove failure must propagate");
        assert!(matches!(err, HalError::Com0Com(ref msg) if msg.contains("remove denied")));
        // Pair remains registered because kernel removal failed.
        assert_eq!(controller.list_pairs().unwrap().len(), 1);
    }
}
