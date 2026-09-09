use anyhow::{Context, Result};
use serde::Deserialize;
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    process::Command,
};
use zip::ZipArchive;

use crate::{local_paths, tui};

use super::{
    process::{npm_executable, run_logged_command},
    release::{download_release_archive, latest_release_asset},
};

const STUDIO_DIR_NAME: &str = "studio";

#[derive(Debug, Deserialize)]
struct StudioPackageManifest {
    version: String,
}

pub(super) fn studio_install_root() -> Result<PathBuf> {
    Ok(local_paths::nautilus_home()?.join(STUDIO_DIR_NAME))
}

pub(super) fn install_or_update_from_release(
    install_root: &Path,
    app_dir: &Path,
    archive_path: &Path,
) -> Result<()> {
    tui::print_section("Studio Download");

    std::fs::create_dir_all(install_root)
        .with_context(|| format!("Failed to create {}", install_root.display()))?;

    if app_dir.exists() {
        std::fs::remove_dir_all(app_dir)
            .with_context(|| format!("Failed to clear {}", app_dir.display()))?;
    }

    if archive_path.exists() {
        std::fs::remove_file(archive_path)
            .with_context(|| format!("Failed to remove {}", archive_path.display()))?;
    }

    let asset = latest_release_asset()?;
    download_release_archive(&asset.browser_download_url, archive_path)?;
    extract_release_archive(archive_path, app_dir)?;

    let app_root = resolve_app_root(app_dir).ok_or_else(|| {
        anyhow::anyhow!(
            "Studio release extracted successfully, but no package.json was found under {}",
            app_dir.display()
        )
    })?;

    install_runtime_dependencies(&app_root)?;

    tui::print_summary_ok("Studio ready", &format!("Downloaded {}", asset.name));
    Ok(())
}

pub(super) fn read_installed_version(app_root: &Path) -> Option<String> {
    read_app_package_version(app_root)
}

fn read_app_package_version(app_root: &Path) -> Option<String> {
    let manifest = std::fs::read_to_string(app_root.join("package.json")).ok()?;
    let package: StudioPackageManifest = serde_json::from_str(&manifest).ok()?;
    let version = package.version.trim().to_string();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

pub(super) fn uninstall_installation(install_root: &Path) -> Result<()> {
    tui::print_section("Studio Uninstall");

    if !install_root.exists() {
        tui::print_summary_ok(
            "Studio already absent",
            &format!("{}", install_root.display()),
        );
        return Ok(());
    }

    std::fs::remove_dir_all(install_root)
        .with_context(|| format!("Failed to remove {}", install_root.display()))?;
    tui::print_summary_ok("Studio uninstalled", &format!("{}", install_root.display()));
    Ok(())
}

fn extract_release_archive(archive_path: &Path, app_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(app_dir)
        .with_context(|| format!("Failed to create {}", app_dir.display()))?;

    let file = File::open(archive_path)
        .with_context(|| format!("Failed to open {}", archive_path.display()))?;
    let mut archive = ZipArchive::new(file)
        .with_context(|| format!("Failed to read {}", archive_path.display()))?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .with_context(|| format!("Failed to read ZIP entry {index}"))?;

        let relative = entry
            .enclosed_name()
            .map(|path| path.to_path_buf())
            .ok_or_else(|| anyhow::anyhow!("ZIP archive contains an invalid path"))?;
        let output_path = app_dir.join(relative);

        if entry.is_dir() {
            std::fs::create_dir_all(&output_path)
                .with_context(|| format!("Failed to create {}", output_path.display()))?;
            continue;
        }

        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }

        let mut output = File::create(&output_path)
            .with_context(|| format!("Failed to create {}", output_path.display()))?;
        io::copy(&mut entry, &mut output)
            .with_context(|| format!("Failed to extract {}", output_path.display()))?;
    }

    tui::print_ok(&format!("Extracted to {}", app_dir.display()));
    Ok(())
}

pub(super) fn resolve_app_root(root: &Path) -> Option<PathBuf> {
    if root.join("package.json").is_file() {
        return Some(root.to_path_buf());
    }

    let release_package = root.join("release-package");
    if release_package.join("package.json").is_file() {
        return Some(release_package);
    }

    let mut discovered = Vec::new();
    collect_app_roots(root, &mut discovered);
    discovered.sort_by_key(|path| path.components().count());
    discovered.into_iter().next()
}

fn collect_app_roots(root: &Path, discovered: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.join("package.json").is_file() {
                discovered.push(path);
            } else {
                collect_app_roots(&path, discovered);
            }
        }
    }
}

pub(super) fn runtime_dependencies_installed(app_root: &Path) -> bool {
    app_root.join("node_modules").exists()
}

pub(super) fn install_runtime_dependencies(app_root: &Path) -> Result<()> {
    tui::print_section("Studio Runtime");

    let mut command = Command::new(npm_executable());
    command.current_dir(app_root);

    if app_root.join("package-lock.json").is_file() {
        command.args(["ci", "--omit=dev"]);
    } else {
        command.args(["install", "--omit=dev"]);
    }

    run_logged_command(
        &mut command,
        "Installing Nautilus Studio runtime dependencies",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_app_root_prefers_release_package_directory() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let app_root = temp_dir.path().join("release-package");
        std::fs::create_dir_all(&app_root).expect("create dirs");
        std::fs::write(app_root.join("package.json"), "{}").expect("package.json");

        let resolved = resolve_app_root(temp_dir.path()).expect("app root");
        assert_eq!(resolved, app_root);
    }

    #[test]
    fn resolve_app_root_falls_back_to_recursive_search() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let nested = temp_dir.path().join("artifact").join("bundle");
        std::fs::create_dir_all(&nested).expect("create dirs");
        std::fs::write(nested.join("package.json"), "{}").expect("package.json");

        let resolved = resolve_app_root(temp_dir.path()).expect("app root");
        assert_eq!(resolved, nested);
    }

    #[test]
    fn recursive_app_root_collection_finds_nested_package_json() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let nested = temp_dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).expect("create dirs");
        std::fs::write(nested.join("package.json"), "{}").expect("package.json");

        let mut discovered = Vec::new();
        collect_app_roots(temp_dir.path(), &mut discovered);

        assert_eq!(discovered, vec![nested]);
    }

    #[test]
    fn installed_version_is_read_from_package_manifest() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let app_root = temp_dir.path().join("app");

        std::fs::create_dir_all(&app_root).expect("create dirs");
        std::fs::write(
            app_root.join("package.json"),
            r#"{"name":"nautilus-studio","version":"0.1.0"}"#,
        )
        .expect("package.json");

        let version = read_installed_version(&app_root).expect("version");

        assert_eq!(version, "0.1.0");
    }
}
