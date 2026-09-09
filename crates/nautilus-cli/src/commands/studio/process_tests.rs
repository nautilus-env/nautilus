use super::*;
use std::io::{BufRead, BufReader};
use std::time::Instant;

#[test]
fn next_cli_path_points_to_bundled_next_binary() {
    let root = Path::new("C:/studio/release-package");
    let cli = next_cli_path(root);

    assert_eq!(
        cli,
        PathBuf::from("C:/studio/release-package/node_modules/next/dist/bin/next")
    );
}

#[test]
fn interrupt_exit_codes_are_treated_as_clean_shutdowns() {
    #[cfg(windows)]
    assert!(is_interrupt_exit_code(0xC000_013A_u32 as i32));

    #[cfg(not(windows))]
    assert!(is_interrupt_exit_code(130));

    for code in [0, 1, 2, 127] {
        assert!(!is_interrupt_exit_code(code));
    }
}

#[test]
fn logged_command_distinguishes_success_failure_and_spawn_errors() {
    let executable = std::env::current_exe().expect("test executable");
    let temp = tempfile::tempdir().expect("temporary directory");
    let mut success = Command::new(&executable);
    success.current_dir(temp.path());
    let test_module = module_path!().split_once("::").expect("crate prefix").1;
    success.args([
        "--exact",
        &format!("{test_module}::next_cli_path_points_to_bundled_next_binary"),
    ]);
    run_logged_command(&mut success, "Starting Nautilus Studio").expect("successful child");

    let mut failure = Command::new(&executable);
    failure
        .current_dir(temp.path())
        .arg("--invalid-studio-test-option");
    let error = run_logged_command(&mut failure, "Starting Nautilus Studio")
        .expect_err("unsuccessful child");
    assert!(error
        .to_string()
        .starts_with("Starting Nautilus Studio failed while running `"));

    let error = run_logged_command(
        Command::new(temp.path().join("missing-studio-executable")).current_dir(temp.path()),
        "Starting Nautilus Studio",
    )
    .expect_err("missing executable");
    assert!(error.to_string().starts_with("Failed to run `"));
}

#[test]
fn interrupted_child_is_terminated_and_reaped() {
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new("powershell.exe");
        command.creation_flags(0x0800_0000).args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Console]::WriteLine('ready'); Start-Sleep -Seconds 15",
        ]);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("sh");
        command.args(["-c", "printf 'ready\\n'; exec sleep 15"]);
        command
    };

    let temp = tempfile::tempdir().expect("temporary directory");
    let mut child = command
        .current_dir(temp.path())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start child");
    let mut ready = String::new();
    BufReader::new(child.stdout.take().expect("child stdout"))
        .read_line(&mut ready)
        .expect("read readiness");
    assert_eq!(ready.trim(), "ready");
    assert!(child.try_wait().expect("running child").is_none());

    let interrupted = Arc::new(AtomicBool::new(true));
    let started = Instant::now();
    let status = wait_for_child(&mut child, &interrupted).expect("interrupted child");
    assert!(!status.success());
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(child.try_wait().expect("reaped child"), Some(status));
    terminate_child(&mut child).expect("already exited child");
}
