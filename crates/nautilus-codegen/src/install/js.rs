//! Installing the generated JavaScript package into `node_modules`.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use super::copy_dir_recursive;

/// Copy the package at `output_path` over `node_modules/nautilus`.
pub(crate) fn install(output_path: &str, schema_path: &Path) -> Result<PathBuf> {
    let node_modules = detect_node_modules(schema_path)?;
    let src = Path::new(output_path);
    let dst = node_modules.join("nautilus");

    if dst.exists() {
        fs::remove_dir_all(&dst).with_context(|| {
            format!(
                "Failed to remove existing installation at: {}",
                dst.display()
            )
        })?;
    }

    copy_dir_recursive(src, &dst)?;

    Ok(dst)
}

/// Walk up from `schema_path` until we find a `node_modules` directory.
fn detect_node_modules(schema_path: &Path) -> Result<PathBuf> {
    let mut current = if schema_path.is_file() {
        schema_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Schema path has no parent directory"))?
    } else {
        schema_path
    };

    loop {
        let candidate = current.join("node_modules");
        if candidate.is_dir() {
            return Ok(candidate);
        }
        current = current.parent().ok_or_else(|| {
            anyhow::anyhow!(
                "No node_modules directory found in '{}' or any parent directory.\n\
                Make sure you run 'nautilus generate' from within a Node.js project \
                (i.e. a directory with node_modules).",
                schema_path.display()
            )
        })?;
    }
}
