#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use classpath_surfer::cli;

/// A Gradle project that has been initialized and indexed.
pub struct IndexedProject {
    _temp: tempfile::TempDir,
    pub project_dir: PathBuf,
}

impl IndexedProject {
    /// Path to the Tantivy index directory.
    pub fn index_dir(&self) -> PathBuf {
        self.project_dir.join(".classpath-surfer/index")
    }
}

/// JDK 21 + Gradle 8.14 indexed project, initialized once per test binary.
///
/// All read-only integration tests share this single build. `None` when JDK 21
/// is not available (the test should skip).
pub static INDEXED_JDK21: LazyLock<Option<IndexedProject>> = LazyLock::new(|| {
    let java_home = get_java_home("21")?;
    let temp = tempfile::tempdir().ok()?;
    let project_dir = copy_fixture_project(temp.path());
    set_gradle_version(&project_dir, "8.14");
    cli::init::run(&project_dir).ok()?;
    refresh_with_java_home(&project_dir, &java_home);
    Some(IndexedProject {
        _temp: temp,
        project_dir,
    })
});

/// Get the shared JDK 21 indexed project, or skip the test if unavailable.
macro_rules! require_indexed_project {
    () => {
        match crate::common::INDEXED_JDK21.as_ref() {
            Some(p) => p,
            None => {
                eprintln!("JDK 21 not available, skipping test");
                return;
            }
        }
    };
}

/// Skip the current test if the required JDK version is not available.
macro_rules! require_jdk {
    ($version:expr) => {
        match crate::common::get_java_home($version) {
            Some(home) => home,
            None => {
                eprintln!("JDK {} not available, skipping test", $version);
                return;
            }
        }
    };
}

pub(crate) use require_indexed_project;
pub(crate) use require_jdk;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gradle-project")
}

/// Resolve JAVA_HOME for a given JDK major version.
///
/// Resolution order:
/// 1. `JAVA_{version}_HOME` env var (e.g. `JAVA_17_HOME`) -- for CI
/// 2. `mise where java@temurin-{version}` -- version-specific local dev lookup
/// 3. `JAVA_HOME` env var -- single-JDK environments (fallback)
/// 4. `None` -- caller decides to skip
pub fn get_java_home(version: &str) -> Option<PathBuf> {
    // 1. JAVA_{version}_HOME
    if let Ok(home) = std::env::var(format!("JAVA_{version}_HOME")) {
        let path = PathBuf::from(home);
        if path.is_dir() {
            return Some(path);
        }
    }

    // 2. mise (version-specific)
    if let Ok(output) = Command::new("mise")
        .args(["where", &format!("java@temurin-{version}")])
        .output()
        && output.status.success()
    {
        let path = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
        if path.is_dir() {
            return Some(path);
        }
    }

    None
}

/// Recursively copy a directory tree.
pub fn copy_dir_recursive(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let dest_path = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path);
        } else {
            std::fs::copy(entry.path(), &dest_path).unwrap();
        }
    }
}

/// Copy the fixture project into a temporary directory and return the canonical path.
///
/// Canonicalizing ensures the path matches what the CLI binary sees
/// (it also canonicalizes via `std::fs::canonicalize`), which prevents
/// macOS `/var` → `/private/var` symlink mismatches in staleness checks.
pub fn copy_fixture_project(temp: &Path) -> PathBuf {
    let canonical_temp = std::fs::canonicalize(temp).expect("temp dir should be canonicalizable");
    let project_dir = canonical_temp.join("project");
    copy_dir_recursive(&fixture_dir(), &project_dir);

    // Ensure gradlew is executable (may lose permission on copy)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let gradlew = project_dir.join("gradlew");
        if gradlew.exists() {
            let mut perms = std::fs::metadata(&gradlew).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&gradlew, perms).unwrap();
        }
    }

    project_dir
}

/// Rewrite gradle-wrapper.properties to use the given Gradle version.
pub fn set_gradle_version(project_dir: &Path, version: &str) {
    let props_path = project_dir.join("gradle/wrapper/gradle-wrapper.properties");
    let content = std::fs::read_to_string(&props_path).unwrap();
    let new_content = content
        .lines()
        .map(|line| {
            if line.starts_with("distributionUrl=") {
                format!(
                    "distributionUrl=https\\://services.gradle.org/distributions/gradle-{version}-bin.zip"
                )
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&props_path, new_content).unwrap();
}

/// Run refresh with a specific JAVA_HOME (full mode).
pub fn refresh_with_java_home(project_dir: &Path, java_home: &Path) {
    let configs = default_configurations();
    cli::refresh::run_with_java_home(project_dir, &configs, true, Some(java_home))
        .expect("refresh should succeed");
}

/// Load the classpath manifest from a project directory.
pub fn read_manifest(project_dir: &Path) -> classpath_surfer::manifest::ClasspathManifest {
    let path = project_dir.join(".classpath-surfer/classpath-manifest.json");
    let content = std::fs::read_to_string(&path).expect("manifest should exist after refresh");
    serde_json::from_str(&content).expect("manifest should be valid JSON")
}

pub fn default_configurations() -> Vec<String> {
    vec![
        "compileClasspath".to_string(),
        "runtimeClasspath".to_string(),
    ]
}

/// Build a CLI command pre-configured with `--project-dir`.
pub fn cli_cmd(project_dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_classpath-surfer"));
    cmd.arg("--project-dir").arg(project_dir);
    cmd
}

/// Create a fresh project copy with init (no refresh). Caller owns the TempDir.
pub fn fresh_project(gradle_version: &str) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = copy_fixture_project(temp.path());
    set_gradle_version(&project_dir, gradle_version);
    cli::init::run(&project_dir).unwrap();
    (temp, project_dir)
}
