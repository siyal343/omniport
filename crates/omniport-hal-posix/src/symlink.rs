use crate::error::{HalError, Result};
use std::path::{Path, PathBuf};

/// RAII guard that removes a symbolic link when dropped
#[derive(Debug)]
pub struct SymlinkGuard {
    target: PathBuf,
    link_path: PathBuf,
    active: bool,
}

impl SymlinkGuard {
    pub fn new(target: PathBuf, link_path: PathBuf) -> Self {
        Self {
            target,
            link_path,
            active: true,
        }
    }

    pub fn target(&self) -> &Path {
        &self.target
    }

    pub fn link_path(&self) -> &Path {
        &self.link_path
    }

    /// Explicitly remove the symlink and consume the guard
    pub fn remove(mut self) -> Result<()> {
        self.cleanup()?;
        self.active = false;
        Ok(())
    }

    fn cleanup(&mut self) -> Result<()> {
        if self.active && (self.link_path.exists() || is_symlink_path(&self.link_path)) {
            std::fs::remove_file(&self.link_path).map_err(|e| HalError::Symlink {
                link: self.link_path.clone(),
                target: self.target.clone(),
                source: e,
            })?;
        }
        Ok(())
    }
}

impl Drop for SymlinkGuard {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn is_symlink_path(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Symlink manager responsible for creating and cleaning up PTY symlinks
pub struct SymlinkManager;

impl SymlinkManager {
    /// Creates a symbolic link at `link_path` pointing to `target`.
    /// Stale files or links at `link_path` are automatically unlinked.
    pub fn create(target: &Path, link_path: &Path) -> Result<SymlinkGuard> {
        // Remove stale link if present
        if link_path.exists() || is_symlink_path(link_path) {
            let _ = std::fs::remove_file(link_path);
        }

        // Ensure parent directory exists
        if let Some(parent) = link_path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent).map_err(|e| HalError::Symlink {
                link: link_path.to_path_buf(),
                target: target.to_path_buf(),
                source: e,
            })?;
        }

        create_symlink_impl(target, link_path)?;

        Ok(SymlinkGuard::new(
            target.to_path_buf(),
            link_path.to_path_buf(),
        ))
    }
}

#[cfg(unix)]
fn create_symlink_impl(target: &Path, link_path: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, link_path).map_err(|source| HalError::Symlink {
        link: link_path.to_path_buf(),
        target: target.to_path_buf(),
        source,
    })
}

#[cfg(not(unix))]
fn create_symlink_impl(target: &Path, link_path: &Path) -> Result<()> {
    // Attempt standard Windows symlink first
    match std::os::windows::fs::symlink_file(target, link_path) {
        Ok(()) => Ok(()),
        Err(_) => {
            // Fallback for unprivileged Windows dev environments: write target marker
            std::fs::write(link_path, target.to_string_lossy().as_bytes()).map_err(|source| {
                HalError::Symlink {
                    link: link_path.to_path_buf(),
                    target: target.to_path_buf(),
                    source,
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symlink_creation_and_raii_cleanup() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let target = temp_dir.path().join("mock_slave");
        let link = temp_dir.path().join("mock_link");

        std::fs::write(&target, b"test-pty").expect("write target");

        {
            let guard = SymlinkManager::create(&target, &link).expect("create symlink");
            assert_eq!(guard.target(), target.as_path());
            assert_eq!(guard.link_path(), link.as_path());
            assert!(link.exists());
        }

        // Guard dropped: link should be automatically cleaned up
        assert!(!link.exists());
        assert!(!is_symlink_path(&link));
        // Target file should still exist
        assert!(target.exists());
    }

    #[test]
    fn test_symlink_explicit_removal() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let target = temp_dir.path().join("mock_slave2");
        let link = temp_dir.path().join("mock_link2");

        std::fs::write(&target, b"test-pty-2").expect("write target");

        let guard = SymlinkManager::create(&target, &link).expect("create symlink");
        assert!(link.exists());

        guard.remove().expect("explicit remove");
        assert!(!link.exists());
    }

    #[test]
    fn test_symlink_overwrite_stale() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let target1 = temp_dir.path().join("slave1");
        let target2 = temp_dir.path().join("slave2");
        let link = temp_dir.path().join("reusable_link");

        std::fs::write(&target1, b"slave1").expect("write target1");
        std::fs::write(&target2, b"slave2").expect("write target2");

        let _guard1 = SymlinkManager::create(&target1, &link).expect("create link 1");
        assert!(link.exists());

        // Overwrite without dropping guard1 first
        let guard2 = SymlinkManager::create(&target2, &link).expect("overwrite link");
        assert!(link.exists());
        assert_eq!(guard2.target(), target2.as_path());
    }
}
