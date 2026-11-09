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
        self.cleanup()?;
        self.active = false;
        Ok(())
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

#[derive(Debug, Default)]
struct Com0ComControllerInner {
    pairs: HashMap<u32, VirtualPair>,
    setupc_path: Option<PathBuf>,
}

impl Com0ComControllerInner {
    fn new() -> Self {
        let setupc_path = find_setupc_binary();
        Self {
            pairs: HashMap::new(),
            setupc_path,
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
        let pair_id = NEXT_PAIR_ID.fetch_add(1, Ordering::SeqCst);
        let mut is_hardware = false;

        if let Some(ref setupc) = self.setupc_path {
            // Invoke setupc.exe install PortName=port_a PortName=port_b
            let status = std::process::Command::new(setupc)
                .args([
                    "install",
                    &format!("PortName={}", port_a),
                    &format!("PortName={}", port_b),
                ])
                .output()
                .map_err(|e| HalError::Com0Com(format!("failed to execute setupc.exe: {}", e)))?;

            if !status.status.success() {
                let err_msg = String::from_utf8_lossy(&status.stderr);
                return Err(HalError::Com0Com(format!(
                    "setupc.exe install failed: {}",
                    err_msg
                )));
            }
            is_hardware = true;
        }

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
        if let Some(pair) = self.pairs.remove(&id)
            && pair.is_hardware
            && let Some(ref setupc) = self.setupc_path
        {
            let _ = std::process::Command::new(setupc)
                .args(["remove", &id.to_string()])
                .output();
        }
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
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Com0ComControllerInner::new())),
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
        let controller = Com0ComController::new();

        let (port_a, port_b) = controller.find_free_pair_names(20).unwrap();
        assert_eq!(port_a, "COM20");
        assert_eq!(port_b, "COM21");

        {
            let guard = controller.create_pair(&port_a, &port_b).unwrap();
            assert_eq!(guard.port_a(), "COM20");
            assert_eq!(guard.port_b(), "COM21");

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
        let controller = Com0ComController::new();

        let guard = controller.create_pair("COM30", "COM31").unwrap();
        assert_eq!(controller.list_pairs().unwrap().len(), 1);

        guard.remove().unwrap();
        assert_eq!(controller.list_pairs().unwrap().len(), 0);
    }
}
