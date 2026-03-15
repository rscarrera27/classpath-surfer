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
    assert_eq!(json["query"], "ImmutableList");
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
fn agentic_deps_json_output() {
    let project = require_indexed_project!();
    let output = Command::new(env!("CARGO_BIN_EXE_classpath-surfer"))
        .args([
            "deps",
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
    assert!(json["total_count"].as_u64().unwrap() > 0);
    assert!(json["dependencies"].as_array().unwrap().len() > 0);
    let first = &json["dependencies"][0];
    assert!(first["gav"].is_string());
    assert!(first["symbol_count"].is_u64());
}

#[test]
fn agentic_search_dependency_json_output() {
    let project = require_indexed_project!();
    let output = Command::new(env!("CARGO_BIN_EXE_classpath-surfer"))
        .args([
            "search",
            "--dependency",
            "com.google.code.gson:gson:*",
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
    assert!(json["dependency"].is_string());
    assert!(!json["matched_gavs"].as_array().unwrap().is_empty());
    assert!(json["total_matches"].as_u64().unwrap() > 0);
    assert!(!json["results"].as_array().unwrap().is_empty());
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

#[test]
fn agentic_show_focus_json() {
    let project = require_indexed_project!();
    let output = Command::new(env!("CARGO_BIN_EXE_classpath-surfer"))
        .args([
            "show",
            "com.google.gson.Gson.fromJson",
            "--agentic",
            "--no-decompile",
            "--context",
            "10",
            "--project-dir",
            &project.project_dir.to_string_lossy(),
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert_eq!(json["fqn"], "com.google.gson.Gson.fromJson");
    assert_eq!(json["symbol_name"], "fromJson");
    // focus is internal-only (serde(skip)), verify via source_path #L fragment
    let path = json["primary"]["source_path"].as_str().unwrap();
    assert!(
        path.contains("#L"),
        "source_path should have #L fragment: {path}"
    );
    // focus should NOT appear in JSON
    assert!(
        json["primary"]["focus"].is_null(),
        "focus should not be in JSON"
    );
}

#[test]
fn agentic_show_full_flag() {
    let project = require_indexed_project!();
    let output = Command::new(env!("CARGO_BIN_EXE_classpath-surfer"))
        .args([
            "show",
            "com.google.gson.Gson.fromJson",
            "--agentic",
            "--no-decompile",
            "--full",
            "--project-dir",
            &project.project_dir.to_string_lossy(),
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    // --full: source_path should NOT have #L fragment
    let path = json["primary"]["source_path"].as_str().unwrap();
    assert!(
        !path.contains("#L"),
        "full mode should not have #L fragment: {path}"
    );
}
