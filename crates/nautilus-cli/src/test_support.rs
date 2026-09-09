//! Helpers for the few tests that still have to move process-global state.
//!
//! Configuration resolvers take a [`crate::context::environment::CommandEnv`]
//! and need none of this. What remains is for code that reads the working
//! directory itself — client generation writing to a path relative to it.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tokio::sync::{Mutex, MutexGuard};

/// Serialise the tests that switch the process working directory.
pub(crate) async fn lock_working_dir_async() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().await
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

pub(crate) fn sqlite_url(path: &Path) -> String {
    let cleaned = path
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('\\', "/");
    format!("sqlite:{cleaned}")
}
