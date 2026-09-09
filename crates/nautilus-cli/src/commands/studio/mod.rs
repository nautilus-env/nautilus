use anyhow::{bail, Context, Result};
use clap::Args;
use std::path::Path;

use crate::{github_release::same_version, tui};

mod install;
mod process;
mod release;

use install::{
    install_or_update_from_release, install_runtime_dependencies, read_installed_version,
    resolve_app_root, runtime_dependencies_installed, studio_install_root, uninstall_installation,
};
use process::{ensure_command_available, launch_app, npm_executable};
use release::{fetch_latest_release_tag_silently, StudioReleaseUnavailable};

pub static STUDIO_GITHUB_REPO: &str = "nautilus-env/nautilus-orm-studio";
pub static STUDIO_RELEASE_ASSET_PREFIX: &str = "nautilus-orm-studio-";

const STUDIO_INSTALL_DIR_NAME: &str = "app";
const STUDIO_ARCHIVE_FILE_NAME: &str = "release.zip";

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub struct StudioArgs {
    /// Download the latest GitHub release again before starting the app
    #[arg(long)]
    pub update: bool,

    /// Remove the locally cached Studio release files
    #[arg(long)]
    pub uninstall: bool,
}

pub async fn run(args: StudioArgs) -> Result<()> {
    tokio::task::spawn_blocking(move || run_sync(args))
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("Task error: {}", e)))
}

fn run_sync(args: StudioArgs) -> Result<()> {
    validate_args(&args)?;
    tui::print_header("studio");

    let project_dir =
        std::env::current_dir().context("Failed to resolve the current project directory")?;
    let install_root = studio_install_root()?;
    let app_dir = install_root.join(STUDIO_INSTALL_DIR_NAME);
    let archive_path = install_root.join(STUDIO_ARCHIVE_FILE_NAME);

    if args.uninstall {
        uninstall_installation(&install_root)?;
        return Ok(());
    }

    ensure_command_available(
        "node",
        &["--version"],
        "Node.js is required to run Nautilus Studio",
    )?;
    ensure_command_available(
        npm_executable(),
        &["--version"],
        "npm is required to run Nautilus Studio",
    )?;

    let needs_download = args.update || resolve_app_root(&app_dir).is_none();
    if needs_download {
        match install_or_update_from_release(&install_root, &app_dir, &archive_path) {
            Ok(()) => {}
            Err(err) => {
                if let Some(unavailable) = err.downcast_ref::<StudioReleaseUnavailable>() {
                    tui::print_warning(&unavailable.detail);
                    return Ok(());
                }
                return Err(err);
            }
        }
    }

    let app_root = resolve_app_root(&app_dir).ok_or_else(|| {
        anyhow::anyhow!(
            "No Studio app root found under {}. Run `nautilus studio --update` after publishing a valid release.",
            app_dir.display()
        )
    })?;

    if !needs_download {
        check_for_update_tip(&app_root);
    }

    if !runtime_dependencies_installed(&app_root) {
        install_runtime_dependencies(&app_root)?;
    }

    launch_app(&app_root, &project_dir)
}

fn validate_args(args: &StudioArgs) -> Result<()> {
    if args.update && args.uninstall {
        bail!("`nautilus studio` does not support using --update and --uninstall together");
    }
    Ok(())
}

fn check_for_update_tip(app_root: &Path) {
    let installed = match read_installed_version(app_root) {
        Some(v) => v,
        None => {
            tui::print_tip(
                "Could not determine the installed Nautilus Studio version from package.json. Run `nautilus studio --update` to refresh it.",
            );
            return;
        }
    };

    let latest = match fetch_latest_release_tag_silently() {
        Some(v) => v,
        None => return,
    };

    if !same_version(&installed, &latest) {
        tui::print_tip(&format!(
            "A newer version of Nautilus Studio is available ({latest}). Run `nautilus studio --update` to install it."
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_args, StudioArgs};

    #[test]
    fn update_and_uninstall_are_mutually_exclusive() {
        for (update, uninstall) in [(false, false), (true, false), (false, true)] {
            validate_args(&StudioArgs { update, uninstall }).expect("valid arguments");
        }

        let error = validate_args(&StudioArgs {
            update: true,
            uninstall: true,
        })
        .expect_err("conflicting arguments");
        assert_eq!(
            error.to_string(),
            "`nautilus studio` does not support using --update and --uninstall together"
        );
    }
}
