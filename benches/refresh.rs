use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};

use classpath_surfer::cli;

fn get_java_home(version: &str) -> Option<PathBuf> {
    if let Ok(home) = std::env::var(format!("JAVA_{version}_HOME")) {
        let path = PathBuf::from(home);
        if path.is_dir() {
            return Some(path);
        }
    }
    if let Ok(home) = std::env::var("JAVA_HOME") {
        let path = PathBuf::from(home);
        if path.is_dir() {
            return Some(path);
        }
    }
    if let Ok(output) = Command::new("mise")
        .args(["where", &format!("java@temurin-{version}")])
        .output()
    {
        if output.status.success() {
            let path = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
            if path.is_dir() {
                return Some(path);
            }
        }
    }
    None
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gradle-project")
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
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

fn copy_fixture_project(temp: &Path) -> PathBuf {
    let project_dir = temp.join("project");
    copy_dir_recursive(&fixture_dir(), &project_dir);
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

fn default_configurations() -> Vec<String> {
    vec![
        "compileClasspath".to_string(),
        "runtimeClasspath".to_string(),
    ]
}

fn bench_refresh(c: &mut Criterion) {
    let Some(java_home) = get_java_home("21") else {
        eprintln!("JDK 21 not available, skipping refresh benchmarks");
        return;
    };

    let mut group = c.benchmark_group("refresh");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(120));

    // Full refresh: clean state → init → refresh
    group.bench_function("full", |b| {
        b.iter_with_setup(
            || {
                let temp = tempfile::tempdir().unwrap();
                let project_dir = copy_fixture_project(temp.path());
                cli::init::run(&project_dir).unwrap();
                (temp, project_dir)
            },
            |(_temp, project_dir)| {
                let configs = default_configurations();
                cli::refresh::run_with_java_home(
                    &project_dir,
                    &configs,
                    true,
                    Some(&java_home),
                    300,
                )
                .unwrap();
            },
        );
    });

    // Incremental refresh: one dependency removed, re-refresh
    group.bench_function("incremental", |b| {
        b.iter_with_setup(
            || {
                let temp = tempfile::tempdir().unwrap();
                let project_dir = copy_fixture_project(temp.path());
                cli::init::run(&project_dir).unwrap();
                let configs = default_configurations();
                cli::refresh::run_with_java_home(
                    &project_dir,
                    &configs,
                    true,
                    Some(&java_home),
                    300,
                )
                .unwrap();
                // Remove gson to trigger incremental diff
                let app_build = project_dir.join("app/build.gradle");
                let content = std::fs::read_to_string(&app_build).unwrap();
                let new_content = content
                    .lines()
                    .filter(|l| !l.contains("gson"))
                    .collect::<Vec<_>>()
                    .join("\n");
                std::fs::write(&app_build, new_content).unwrap();
                (temp, project_dir, configs)
            },
            |(_temp, project_dir, configs)| {
                cli::refresh::run_with_java_home(
                    &project_dir,
                    &configs,
                    false,
                    Some(&java_home),
                    300,
                )
                .unwrap();
            },
        );
    });

    // Noop refresh: already up-to-date, no changes
    group.bench_function("noop", |b| {
        // Setup once: create indexed project
        let temp = tempfile::tempdir().unwrap();
        let project_dir = copy_fixture_project(temp.path());
        cli::init::run(&project_dir).unwrap();
        let configs = default_configurations();
        cli::refresh::run_with_java_home(&project_dir, &configs, true, Some(&java_home), 300)
            .unwrap();

        b.iter(|| {
            cli::refresh::run_with_java_home(&project_dir, &configs, false, Some(&java_home), 300)
                .unwrap();
        });
    });

    group.finish();
}

criterion_group!(benches, bench_refresh);
criterion_main!(benches);
