//! Output mode detection and JSON emission helpers.
//!
//! Determines whether the CLI should emit structured JSON (`--agentic`),
//! render an interactive TUI (TTY detected), or fall back to plain text.

use std::io::IsTerminal;

use serde::Serialize;

/// How the CLI should render its output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Structured JSON to stdout (for AI agents / scripts).
    Agentic,
    /// Interactive ratatui alternate-screen TUI (TTY detected).
    Tui,
    /// Plain text tables / messages (non-TTY pipe or redirect).
    Plain,
}

impl OutputMode {
    /// Decide the output mode from CLI flags and TTY state.
    ///
    /// - `--agentic` / `--json` → [`OutputMode::Agentic`]
    /// - `--plain`, `--no-color`, `NO_COLOR` env, `TERM=dumb` → [`OutputMode::Plain`]
    /// - stdout is a TTY → [`OutputMode::Tui`]
    /// - otherwise → [`OutputMode::Plain`]
    pub fn detect(agentic: bool, plain: bool, no_color: bool) -> Self {
        if agentic {
            return Self::Agentic;
        }
        if plain || no_color || color_disabled_by_env() {
            return Self::Plain;
        }
        if std::io::stdout().is_terminal() {
            Self::Tui
        } else {
            Self::Plain
        }
    }
}

/// Check if color is disabled via environment variables (`NO_COLOR` or `TERM=dumb`).
fn color_disabled_by_env() -> bool {
    std::env::var_os("NO_COLOR").is_some() || std::env::var("TERM").ok().as_deref() == Some("dumb")
}

/// Serialize `value` as pretty-printed JSON and write it to stdout.
pub fn emit_json<T: Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Emit a JSON error object to stdout and exit with the appropriate code.
///
/// If the error chain contains a [`crate::error::CliError`], its `error_code`,
/// `exit_code`, and `retryable` fields are extracted; otherwise defaults to
/// `UNKNOWN` / exit 1 / not retryable.
pub fn emit_json_error(error: &anyhow::Error) -> ! {
    let cli_err = error.downcast_ref::<crate::error::CliError>();
    let error_code = cli_err.map(|e| e.error_code).unwrap_or("UNKNOWN");
    let exit_code = cli_err.map(|e| e.exit_code).unwrap_or(1);
    let retryable = cli_err.map(|e| e.retryable).unwrap_or(false);
    let suggested_command = cli_err.and_then(|e| e.suggested_command.as_deref());

    let mut json = serde_json::json!({
        "success": false,
        "error_code": error_code,
        "error": format!("{error:#}"),
        "retryable": retryable,
    });
    if let Some(cmd) = suggested_command {
        json["suggested_command"] = serde_json::Value::String(cmd.to_string());
    }
    println!("{}", serde_json::to_string_pretty(&json).unwrap());
    std::process::exit(exit_code);
}
