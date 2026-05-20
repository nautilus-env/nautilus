use anyhow::{anyhow, Result};
use std::{env, path::PathBuf};

pub(crate) fn nautilus_home() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Ok(base) = env::var("LOCALAPPDATA") {
            return Ok(PathBuf::from(base).join("nautilus"));
        }
        Ok(home_dir()?.join(".nautilus"))
    }

    #[cfg(not(target_os = "windows"))]
    {
        Ok(home_dir()?.join(".nautilus"))
    }
}

fn home_dir() -> Result<PathBuf> {
    env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .map_err(|_| anyhow!("Could not determine home directory (HOME / USERPROFILE not set)"))
}
