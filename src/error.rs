//! Classified CLI error type with exit code, error code, and retryability.

use std::fmt;

/// A classified CLI error carrying structured metadata for agent-friendly diagnostics.
///
/// Each variant encodes an `error_code` (machine-readable), an `exit_code` (for
/// process exit), and a `retryable` flag so that callers (especially AI agents)
/// can branch on the failure category without parsing human-readable text.
///
/// `CliError` is designed to be wrapped inside [`anyhow::Error`] — handler
/// return types stay `anyhow::Result<T>`, and the top-level error path
/// downcasts to extract the metadata.
#[derive(Debug)]
pub struct CliError {
    /// Machine-readable error code (e.g. `INDEX_NOT_FOUND`).
    pub error_code: &'static str,
    /// Process exit code: 1 = general, 2 = usage, 3 = resource not found.
    pub exit_code: i32,
    /// Human-readable error message.
    pub message: String,
    /// Whether the operation may succeed if retried (e.g. transient network / process failures).
    pub retryable: bool,
    /// Suggested command for the agent to run to recover from this error.
    pub suggested_command: Option<String>,
}

impl CliError {
    /// Resource not found (exit 3, not retryable).
    pub fn resource_not_found(error_code: &'static str, msg: impl Into<String>) -> Self {
        Self {
            error_code,
            exit_code: 3,
            message: msg.into(),
            retryable: false,
            suggested_command: None,
        }
    }

    /// General failure (exit 1, not retryable).
    pub fn general(error_code: &'static str, msg: impl Into<String>) -> Self {
        Self {
            error_code,
            exit_code: 1,
            message: msg.into(),
            retryable: false,
            suggested_command: None,
        }
    }

    /// Usage error — bad arguments (exit 2, not retryable).
    pub fn usage(error_code: &'static str, msg: impl Into<String>) -> Self {
        Self {
            error_code,
            exit_code: 2,
            message: msg.into(),
            retryable: false,
            suggested_command: None,
        }
    }

    /// Transient failure that may succeed on retry (exit 1, retryable).
    pub fn transient(error_code: &'static str, msg: impl Into<String>) -> Self {
        Self {
            error_code,
            exit_code: 1,
            message: msg.into(),
            retryable: true,
            suggested_command: None,
        }
    }

    /// Attach a suggested recovery command for agent consumption.
    pub fn with_suggested_command(mut self, cmd: impl Into<String>) -> Self {
        self.suggested_command = Some(cmd.into());
        self
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}
