use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tokio::sync::{Mutex, MutexGuard};

fn working_dir_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) fn lock_working_dir() -> MutexGuard<'static, ()> {
    working_dir_lock().blocking_lock()
}

pub(crate) async fn lock_working_dir_async() -> MutexGuard<'static, ()> {
    working_dir_lock().lock().await
}

pub(crate) struct CurrentDirGuard {
    original: PathBuf,
}

impl CurrentDirGuard {
    pub(crate) fn set(path: &Path) -> Self {
        let original = std::env::current_dir().expect("current dir should exist");
        std::env::set_current_dir(path).expect("failed to switch current dir");
        Self { original }
    }
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.original).expect("failed to restore current dir");
    }
}

pub(crate) struct EnvVarGuard {
    key: String,
    original: Option<String>,
}

impl EnvVarGuard {
    pub(crate) fn unset(key: &str) -> Self {
        let original = std::env::var(key).ok();
        std::env::remove_var(key);
        Self {
            key: key.to_string(),
            original,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(value) = &self.original {
            std::env::set_var(&self.key, value);
        } else {
            std::env::remove_var(&self.key);
        }
    }
}

pub(crate) fn sqlite_url(path: &Path) -> String {
    let cleaned = path
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('\\', "/");
    format!("sqlite:{cleaned}")
}
