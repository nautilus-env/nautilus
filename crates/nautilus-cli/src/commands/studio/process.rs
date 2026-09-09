use anyhow::{bail, Context, Result};
use std::{
    io,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    time::Duration,
};

use crate::tui;

use super::STUDIO_GITHUB_REPO;

pub(super) fn launch_app(app_root: &Path, project_dir: &Path) -> Result<()> {
    tui::print_section("Studio Start");
    tui::print_ok(&format!(
        "Release repo: https://github.com/{}",
        STUDIO_GITHUB_REPO
    ));
    tui::print_ok(&format!("Project directory: {}", project_dir.display()));
    tui::print_ok(&format!("Studio directory: {}", app_root.display()));

    let next_cli = next_cli_path(app_root);
    if !next_cli.is_file() {
        bail!(
            "Could not find the bundled Next.js CLI at {}",
            next_cli.display()
        );
    }

    let mut command = Command::new("node");
    command
        .current_dir(project_dir)
        .arg(&next_cli)
        .arg("start")
        .arg(app_root);

    run_logged_command(&mut command, "Starting Nautilus Studio")
}

fn next_cli_path(app_root: &Path) -> PathBuf {
    app_root
        .join("node_modules")
        .join("next")
        .join("dist")
        .join("bin")
        .join("next")
}

pub(super) fn npm_executable() -> &'static str {
    if cfg!(windows) {
        "npm.cmd"
    } else {
        "npm"
    }
}

pub(super) fn ensure_command_available(program: &str, args: &[&str], message: &str) -> Result<()> {
    let available = Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);

    if !available {
        bail!("{message}");
    }

    Ok(())
}

pub(super) fn run_logged_command(command: &mut Command, description: &str) -> Result<()> {
    let rendered = format_command(command);
    let interrupted = studio_interrupt_flag()?;
    interrupted.store(false, Ordering::SeqCst);

    let mut child = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("Failed to run `{rendered}`"))?;

    let status = wait_for_child(&mut child, &interrupted)
        .with_context(|| format!("Failed while running `{rendered}`"))?;

    if interrupted.load(Ordering::SeqCst) || is_interrupt_exit_status(&status) {
        let display_name = description
            .strip_prefix("Starting ")
            .unwrap_or(description)
            .to_string();
        tui::print_summary_ok(&format!("{display_name} stopped"), "Interrupted by Ctrl+C");
        return Ok(());
    }

    if !status.success() {
        bail!("{description} failed while running `{rendered}`");
    }

    Ok(())
}

fn studio_interrupt_flag() -> Result<Arc<AtomicBool>> {
    static FLAG: OnceLock<std::result::Result<Arc<AtomicBool>, String>> = OnceLock::new();

    match FLAG.get_or_init(|| {
        let flag = Arc::new(AtomicBool::new(false));
        let handler_flag = Arc::clone(&flag);

        ctrlc::set_handler(move || {
            handler_flag.store(true, Ordering::SeqCst);
        })
        .map(|_| flag)
        .map_err(|error| error.to_string())
    }) {
        Ok(flag) => Ok(Arc::clone(flag)),
        Err(error) => Err(anyhow::anyhow!(
            "Failed to install Ctrl+C handler for Nautilus Studio: {}",
            error
        )),
    }
}

fn wait_for_child(child: &mut Child, interrupted: &Arc<AtomicBool>) -> Result<ExitStatus> {
    loop {
        if let Some(status) = child.try_wait().context("Failed to query child status")? {
            return Ok(status);
        }

        if interrupted.load(Ordering::SeqCst) {
            terminate_child(child)?;
            return child.wait().context("Failed to wait for interrupted child");
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

fn terminate_child(child: &mut Child) -> Result<()> {
    if child
        .try_wait()
        .context("Failed to query child status before termination")?
        .is_some()
    {
        return Ok(());
    }

    match child.kill() {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => Ok(()),
        Err(error) => Err(error).context("Failed to terminate interrupted child process"),
    }
}

fn is_interrupt_exit_status(status: &ExitStatus) -> bool {
    if let Some(code) = status.code() {
        return is_interrupt_exit_code(code);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.signal() == Some(2)
    }

    #[cfg(not(unix))]
    {
        false
    }
}

fn is_interrupt_exit_code(code: i32) -> bool {
    #[cfg(windows)]
    {
        code == 0xC000_013A_u32 as i32
    }

    #[cfg(not(windows))]
    {
        code == 130
    }
}

fn format_command(command: &Command) -> String {
    let program = command.get_program().to_string_lossy();
    let args = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    if args.is_empty() {
        program.into_owned()
    } else {
        format!("{program} {}", args.join(" "))
    }
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
