//! Making a generated Rust crate part of the caller's Cargo workspace.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Add the generated crate to the workspace `Cargo.toml` `[members]` array
/// (analogous to installing the Python package for the Python provider).
///
/// Walks up from `schema_path` until it finds a `Cargo.toml` that contains
/// `[workspace]`. The member entry is expressed as a path relative to that
/// workspace root so the result stays portable.
pub(crate) fn integrate_package(output_path: &str, schema_path: &Path) -> Result<()> {
    use std::io::Write;

    let workspace_toml_path = find_workspace_cargo_toml(schema_path).ok_or_else(|| {
        anyhow::anyhow!(
            "No workspace Cargo.toml found in '{}' or any parent directory.\n\
            Make sure you run 'nautilus generate' from within a Cargo workspace.",
            schema_path.display()
        )
    })?;

    let mut content =
        fs::read_to_string(&workspace_toml_path).context("Failed to read workspace Cargo.toml")?;

    let workspace_dir = workspace_toml_path.parent().unwrap();

    // Resolve the output path to an absolute path (it may be relative to cwd).
    let output_absolute = if Path::new(output_path).is_absolute() {
        PathBuf::from(output_path)
    } else {
        std::env::current_dir()
            .context("Failed to get current directory")?
            .join(output_path)
    };
    // Strip the Windows \\?\ UNC prefix when present.
    let cleaned_output = {
        let s = output_absolute.to_string_lossy();
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            PathBuf::from(stripped)
        } else {
            output_absolute.clone()
        }
    };

    let member_path: String = if let Ok(rel) = cleaned_output.strip_prefix(workspace_dir) {
        rel.to_string_lossy().replace('\\', "/")
    } else {
        // Fall back to the absolute path (unusual, but don't panic).
        cleaned_output.to_string_lossy().replace('\\', "/")
    };

    if content.contains(&member_path) {
    } else {
        // Find the closing bracket of the `members = [...]` array and insert
        // our entry before it. We handle both single-line and multi-line forms.
        //
        // Strategy: find "members" key, then find the matching `]` and inject.
        if let Some(members_pos) = content.find("members") {
            // Find the `[` that opens the array.
            if let Some(bracket_open) = content[members_pos..].find('[') {
                let open_abs = members_pos + bracket_open;
                // Find the matching `]`.
                if let Some(bracket_close) = content[open_abs..].find(']') {
                    let close_abs = open_abs + bracket_close;
                    // Insert before the closing bracket, with a trailing comma.
                    let insert = format!(",\n    \"{}\"", member_path);
                    // If the array is empty we don't want a leading comma.
                    let inner = content[open_abs + 1..close_abs].trim();
                    let insert = if inner.is_empty() {
                        format!("\n    \"{}\"", member_path)
                    } else {
                        insert
                    };
                    content.insert_str(close_abs, &insert);
                }
            }
        } else {
            // No `members` key at all — append a new one.
            content.push_str(&format!("\nmembers = [\n    \"{}\"]\n", member_path));
        }

        let mut file = fs::File::create(&workspace_toml_path)
            .context("Failed to open workspace Cargo.toml for writing")?;
        file.write_all(content.as_bytes())
            .context("Failed to write workspace Cargo.toml")?;
    }

    Ok(())
}

/// Walk up from `start` until we find a `Cargo.toml` that contains `[workspace]`.
pub(crate) fn find_workspace_cargo_toml(start: &Path) -> Option<PathBuf> {
    let mut current = if start.is_file() {
        start.parent()?
    } else {
        start
    };
    loop {
        let candidate = current.join("Cargo.toml");
        if candidate.exists() {
            if let Ok(content) = fs::read_to_string(&candidate) {
                if content.contains("[workspace]") {
                    return Some(candidate);
                }
            }
        }
        current = current.parent()?;
    }
}
