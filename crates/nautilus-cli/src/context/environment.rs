//! The process state the configuration of a command depends on.
//!
//! Where the command runs and what the environment says are read once, at the
//! entry point, and passed down as a value. The resolvers stay pure functions
//! of that value, so a test hands them a directory and a set of variables
//! instead of moving the whole process into a temporary directory.

use std::path::{Path, PathBuf};

/// Where variables are read from, and where `.env` entries are written to.
enum Variables {
    /// The process environment: reads see exported variables, and a `.env`
    /// entry is exported so anything the command starts inherits it.
    Process,
    /// A set of variables held by this value alone, touching no process state.
    #[cfg(test)]
    Fixed(std::collections::BTreeMap<String, String>),
}

/// The working directory and environment variables a command was invoked with.
pub struct CommandEnv {
    current_dir: PathBuf,
    variables: Variables,
}

impl CommandEnv {
    /// The environment of the running process.
    ///
    /// An unreadable working directory falls back to `.`, the relative path
    /// every other file operation resolves against anyway.
    pub fn from_process() -> Self {
        Self {
            current_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            variables: Variables::Process,
        }
    }

    /// An environment that starts with no variables and belongs to its holder.
    #[cfg(test)]
    pub fn fixed(current_dir: impl Into<PathBuf>) -> Self {
        Self {
            current_dir: current_dir.into(),
            variables: Variables::Fixed(std::collections::BTreeMap::new()),
        }
    }

    /// The directory relative paths resolve against.
    pub fn current_dir(&self) -> &Path {
        &self.current_dir
    }

    /// The value of `key`, or `None` when it is unset.
    pub fn var(&self, key: &str) -> Option<String> {
        match &self.variables {
            Variables::Process => std::env::var(key).ok(),
            #[cfg(test)]
            Variables::Fixed(values) => values.get(key).cloned(),
        }
    }

    /// Set `key` unless it already has a value.
    ///
    /// A shell export always wins over a `.env` entry, which is why an
    /// existing value is never replaced.
    pub fn set_var_if_absent(&mut self, key: &str, value: &str) {
        match &mut self.variables {
            Variables::Process => {
                if std::env::var(key).is_err() {
                    // SAFETY: single-threaded context (before async spawn)
                    #[allow(clippy::disallowed_methods)]
                    std::env::set_var(key, value);
                }
            }
            #[cfg(test)]
            Variables::Fixed(values) => {
                values
                    .entry(key.to_string())
                    .or_insert_with(|| value.to_string());
            }
        }
    }
}
