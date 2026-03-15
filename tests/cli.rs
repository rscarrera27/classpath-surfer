mod common;

use std::process::Command;

use common::require_indexed_project;

// ---------------------------------------------------------------------------
// True E2E tests: run the CLI binary as a subprocess
// ---------------------------------------------------------------------------

#[test]
fn agentic_search_json_output() {
    let project = require_indexed_project!();
    let output = Command::new(env!("CARGO_BIN_EXE_classpath-surfer"))
        .args([
            "search",
            "ImmutableList",
            "--agentic",
            "--project-dir",
            &project.project_dir.to_string_lossy(),
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "exit code should be 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("stdout should be valid JSON in agentic mode");
    assert!(!json["results"].as_array().unwrap().is_empty());
    assert!(json["query"].is_string());
    assert!(json["total_matches"].is_u64());
}

#[test]
fn agentic_error_json_format() {
    let output = Command::new(env!("CARGO_BIN_EXE_classpath-surfer"))
        .args([
            "search",
            "Foo",
            "--agentic",
            "--project-dir",
            "/tmp/nonexistent-classpath-surfer-test",
        ])
        .output()
        .unwrap();

    assert_ne!(output.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("stdout should be valid JSON even on error in agentic mode");
    assert_eq!(json["success"], false);
    assert!(json["error_code"].is_string());
    assert!(json["error"].is_string());
}

#[test]
fn exit_code_for_missing_index() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("empty-project");
    std::fs::create_dir_all(&project_dir).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_classpath-surfer"))
        .args([
            "search",
            "Foo",
            "--project-dir",
            &project_dir.to_string_lossy(),
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(3),
        "missing index should exit with code 3"
    );
}

#[test]
fn plain_output_to_pipe() {
    let project = require_indexed_project!();
    let output = Command::new(env!("CARGO_BIN_EXE_classpath-surfer"))
        .args([
            "search",
            "ImmutableList",
            "--project-dir",
            &project.project_dir.to_string_lossy(),
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    // When piped (non-TTY), output should be plain text, not JSON
    assert!(!stdout.starts_with('{'), "pipe output should not be JSON");
    assert!(
        stdout.contains("ImmutableList"),
        "plain output should contain search term"
    );
}

#[test]
fn access_filter_parsing() {
    let project = require_indexed_project!();
    let output = Command::new(env!("CARGO_BIN_EXE_classpath-surfer"))
        .args([
            "search",
            "ImmutableList",
            "--agentic",
            "--access",
            "public,protected",
            "--project-dir",
            &project.project_dir.to_string_lossy(),
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "comma-separated --access should parse correctly, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("should produce valid JSON");
    assert!(json["results"].is_array());
}

#[test]
fn agentic_status_json_output() {
    let project = require_indexed_project!();
    let output = Command::new(env!("CARGO_BIN_EXE_classpath-surfer"))
        .args([
            "status",
            "--agentic",
            "--project-dir",
            &project.project_dir.to_string_lossy(),
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("status should produce valid JSON");
    assert!(json["initialized"].as_bool().unwrap());
    assert!(json["has_index"].as_bool().unwrap());
    assert!(json["dependency_count"].as_u64().unwrap() > 0);
}

#[test]
fn invalid_project_dir_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_classpath-surfer"))
        .args([
            "search",
            "Foo",
            "--agentic",
            "--project-dir",
            "/tmp/nonexistent-classpath-surfer-test",
        ])
        .output()
        .unwrap();

    assert_ne!(output.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("agentic mode should produce JSON even for invalid project-dir");
    assert_eq!(json["success"], false);
    assert!(json["error_code"].is_string());
}
