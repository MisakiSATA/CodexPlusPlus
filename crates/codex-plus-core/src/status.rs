use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use fs2::FileExt;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LaunchStatus {
    pub status: String,
    pub message: String,
    pub started_at_ms: u64,
    pub debug_port: Option<u16>,
    pub helper_port: Option<u16>,
    pub codex_app: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StatusStore {
    path: PathBuf,
}

impl Default for StatusStore {
    fn default() -> Self {
        Self::new(crate::paths::default_latest_status_path())
    }
}

impl StatusStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Save the latest launch record while serializing writers across launcher processes.
    /// The lock lives beside (rather than inside) the status file because `atomic_write`
    /// replaces the status inode on every update.
    pub fn save_latest(&self, status: &LaunchStatus) -> anyhow::Result<()> {
        self.with_lock(|| self.save_latest_unlocked(status))
    }

    pub fn load_latest(&self) -> anyhow::Result<Option<LaunchStatus>> {
        self.load_latest_unlocked()
    }

    /// Atomically read, transform, and (when requested) write the latest launch record.
    /// This is used for terminal-state updates so an older launcher cannot perform a
    /// load→save sequence that races and clobbers a newer launch.
    pub fn update_latest_if<F>(&self, update: F) -> anyhow::Result<bool>
    where
        F: FnOnce(Option<LaunchStatus>) -> Option<LaunchStatus>,
    {
        self.with_lock(|| {
            let current = self.load_latest_unlocked()?;
            let Some(next) = update(current) else {
                return Ok(false);
            };
            self.save_latest_unlocked(&next)?;
            Ok(true)
        })
    }

    fn save_latest_unlocked(&self, status: &LaunchStatus) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec_pretty(status)?;
        crate::settings::atomic_write(&self.path, &bytes)
    }

    fn load_latest_unlocked(&self) -> anyhow::Result<Option<LaunchStatus>> {
        let contents = match fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to read latest status {}", self.path.display())
                });
            }
        };

        Ok(serde_json::from_str(&contents).ok())
    }

    fn with_lock<T>(&self, operation: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
        let mut lock_path = self.path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let lock_path = PathBuf::from(lock_path);
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
        let lock_file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("failed to open status lock {}", lock_path.display()))?;
        lock_file
            .lock_exclusive()
            .with_context(|| format!("failed to lock status {}", lock_path.display()))?;
        let result = operation();
        let _ = lock_file.unlock();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "codex-plus-core-status-test-{}-{}",
            std::process::id(),
            NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn status_store_save_load_latest_roundtrip_uses_custom_path() {
        let dir = temp_dir();
        let store = StatusStore::new(dir.join("nested").join("latest-status.json"));
        let status = LaunchStatus {
            status: "running".to_string(),
            message: "ready".to_string(),
            started_at_ms: 12345,
            debug_port: Some(9222),
            helper_port: Some(4545),
            codex_app: Some("Codex".to_string()),
        };

        store.save_latest(&status).unwrap();

        assert_eq!(store.load_latest().unwrap(), Some(status));
    }

    #[test]
    fn status_store_update_latest_if_applies_conditional_change_atomically() {
        let dir = temp_dir();
        let store = StatusStore::new(dir.join("latest-status.json"));
        let status = LaunchStatus {
            status: "running".to_string(),
            message: "ready".to_string(),
            started_at_ms: 12345,
            debug_port: Some(9222),
            helper_port: Some(4545),
            codex_app: Some("Codex".to_string()),
        };
        store.save_latest(&status).unwrap();

        let changed = store
            .update_latest_if(|current| {
                let mut current = current.unwrap();
                assert_eq!(current.started_at_ms, 12345);
                current.status = "stopped".to_string();
                Some(current)
            })
            .unwrap();

        assert!(changed);
        assert_eq!(store.load_latest().unwrap().unwrap().status, "stopped");

        let changed = store
            .update_latest_if(|current| {
                assert_eq!(current.unwrap().status, "stopped");
                None
            })
            .unwrap();
        assert!(!changed);
        assert_eq!(store.load_latest().unwrap().unwrap().status, "stopped");
    }

    #[test]
    fn status_store_terminal_update_does_not_race_a_newer_save() {
        use std::sync::{Arc, Barrier};
        use std::thread;
        use std::time::Duration;

        let dir = temp_dir();
        let path = dir.join("latest-status.json");
        let store = StatusStore::new(path);
        let initial = LaunchStatus {
            status: "running".to_string(),
            message: "first".to_string(),
            started_at_ms: 1,
            debug_port: Some(9229),
            helper_port: Some(57321),
            codex_app: Some("Codex".to_string()),
        };
        let newer = LaunchStatus {
            status: "running".to_string(),
            message: "second".to_string(),
            started_at_ms: 2,
            debug_port: Some(9229),
            helper_port: Some(57321),
            codex_app: Some("Codex".to_string()),
        };
        store.save_latest(&initial).unwrap();

        let entered = Arc::new(Barrier::new(2));
        let update_store = store.clone();
        let update_entered = Arc::clone(&entered);
        let updater = thread::spawn(move || {
            update_store
                .update_latest_if(|current| {
                    let mut current = current.unwrap();
                    update_entered.wait();
                    // Keep the callback open long enough for the competing writer to try.
                    thread::sleep(Duration::from_millis(50));
                    current.status = "stopped".to_string();
                    Some(current)
                })
                .unwrap();
        });
        entered.wait();
        // This save must wait for the conditional update's sidecar lock and win afterward.
        store.save_latest(&newer).unwrap();
        updater.join().unwrap();

        assert_eq!(store.load_latest().unwrap(), Some(newer));
    }

    #[test]
    fn status_store_load_latest_missing_file_returns_none() {
        let dir = temp_dir();
        let store = StatusStore::new(dir.join("latest-status.json"));

        assert_eq!(store.load_latest().unwrap(), None);
    }

    #[test]
    fn status_store_load_latest_bad_json_returns_none() {
        let dir = temp_dir();
        let path = dir.join("latest-status.json");
        std::fs::write(&path, "{bad json").unwrap();
        let store = StatusStore::new(path);

        assert_eq!(store.load_latest().unwrap(), None);
    }
}
