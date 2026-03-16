use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::error::CliError;

/// Run the Gradle `classpathSurferExport` task with a timeout.
///
/// If `java_home` is provided, sets `JAVA_HOME` for the Gradle process.
/// The child process is killed on Ctrl-C (SIGINT) or when the timeout expires.
pub fn run_export_task(
    project_dir: &Path,
    init_script_path: &Path,
    configurations: &[String],
    java_home: Option<&Path>,
    timeout_secs: u64,
) -> Result<()> {
    let gradle_cmd = find_gradle(project_dir);

    let configs_prop = configurations.join(",");

    let mut cmd = Command::new(&gradle_cmd);
    cmd.current_dir(project_dir)
        .arg("--init-script")
        .arg(init_script_path)
        .arg(format!("-PclasspathSurfer.configurations={configs_prop}"))
        .arg("classpathSurferExport")
        .arg("--quiet")
        .arg("--parallel");

    if let Some(java_home) = java_home {
        cmd.env("JAVA_HOME", java_home);
    }

    let mut child = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning {gradle_cmd}"))?;

    // Ctrl-C flag — the handler sets this so the main loop can react
    let interrupted = Arc::new(AtomicBool::new(false));
    let interrupted_clone = Arc::clone(&interrupted);
    let _ = ctrlc::set_handler(move || {
        interrupted_clone.store(true, Ordering::SeqCst);
    });

    // Poll child with timeout
    let timeout = Duration::from_secs(timeout_secs);
    let start = std::time::Instant::now();
    loop {
        // Check Ctrl-C
        if interrupted.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            std::process::exit(130);
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    let stderr_output = child
                        .stderr
                        .take()
                        .and_then(|mut s| {
                            let mut buf = String::new();
                            std::io::Read::read_to_string(&mut s, &mut buf).ok()?;
                            Some(buf)
                        })
                        .unwrap_or_default();
                    let exit_code = status.code().unwrap_or(-1);
                    let mut msg = format!("Gradle task failed with exit code: {exit_code}");
                    let stderr_trimmed = stderr_output.trim();
                    if !stderr_trimmed.is_empty() {
                        msg.push_str("\n\nGradle stderr:\n");
                        msg.push_str(stderr_trimmed);
                    }
                    return Err(CliError::transient("GRADLE_FAILED", msg).into());
                }
                return Ok(());
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(CliError::transient(
                        "GRADLE_TIMEOUT",
                        format!(
                            "Gradle task timed out after {timeout_secs} seconds. \
                             Use --timeout to increase the limit."
                        ),
                    )
                    .into());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                return Err(anyhow::Error::new(e).context(format!("waiting for {gradle_cmd}")));
            }
        }
    }
}

fn find_gradle(project_dir: &Path) -> String {
    // Prefer gradlew in the project directory
    let gradlew = if cfg!(windows) {
        project_dir.join("gradlew.bat")
    } else {
        project_dir.join("gradlew")
    };

    if gradlew.exists() {
        gradlew.to_string_lossy().into_owned()
    } else {
        "gradle".to_string()
    }
}
