use std::path::{Path, PathBuf};
use std::process::Command;

use criterion::{Criterion, criterion_group, criterion_main};

use classpath_surfer::cli;
use classpath_surfer::index::reader::IndexReader;
use classpath_surfer::model::SearchQuery;

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

/// Prepare an indexed project directory. Returns the path (kept alive by the TempDir).
fn prepare_indexed_project() -> Option<(tempfile::TempDir, PathBuf)> {
    let java_home = get_java_home("21")?;
    let temp = tempfile::tempdir().unwrap();
    let project_dir = copy_fixture_project(temp.path());

    cli::init::run(&project_dir).ok()?;
    let configs = vec![
        "compileClasspath".to_string(),
        "runtimeClasspath".to_string(),
    ];
    cli::refresh::run_with_java_home(&project_dir, &configs, true, Some(&java_home)).ok()?;

    Some((temp, project_dir))
}

fn bench_search(c: &mut Criterion) {
    let Some((_temp, project_dir)) = prepare_indexed_project() else {
        eprintln!("JDK 21 not available, skipping search benchmarks");
        return;
    };

    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir).expect("index should be readable");

    let mut group = c.benchmark_group("search");

    group.bench_function("simple", |b| {
        b.iter(|| {
            let _ = reader
                .search(&SearchQuery {
                    query: Some("ImmutableList"),
                    symbol_type: "any",
                    fqn_mode: false,
                    regex_mode: false,
                    limit: 20,
                    dependency: None,
                    access_levels: None,
                    offset: 0,
                    scope: None,
                })
                .unwrap();
        });
    });

    group.bench_function("fqn", |b| {
        b.iter(|| {
            let _ = reader
                .search(&SearchQuery {
                    query: Some("com.google.common.collect.ImmutableList"),
                    symbol_type: "any",
                    fqn_mode: true,
                    regex_mode: false,
                    limit: 20,
                    dependency: None,
                    access_levels: None,
                    offset: 0,
                    scope: None,
                })
                .unwrap();
        });
    });

    group.bench_function("regex", |b| {
        b.iter(|| {
            let _ = reader
                .search(&SearchQuery {
                    query: Some("Immutable.*"),
                    symbol_type: "any",
                    fqn_mode: false,
                    regex_mode: true,
                    limit: 20,
                    dependency: None,
                    access_levels: None,
                    offset: 0,
                    scope: None,
                })
                .unwrap();
        });
    });

    group.bench_function("type_filter", |b| {
        b.iter(|| {
            let _ = reader
                .search(&SearchQuery {
                    query: Some("ImmutableList"),
                    symbol_type: "class",
                    fqn_mode: false,
                    regex_mode: false,
                    limit: 20,
                    dependency: None,
                    access_levels: None,
                    offset: 0,
                    scope: None,
                })
                .unwrap();
        });
    });

    group.bench_function("dependency_filter", |b| {
        b.iter(|| {
            let _ = reader
                .search(&SearchQuery {
                    query: Some("ImmutableList"),
                    symbol_type: "any",
                    fqn_mode: false,
                    regex_mode: false,
                    limit: 20,
                    dependency: Some("com.google.guava:guava:33.4.0-jre"),
                    access_levels: None,
                    offset: 0,
                    scope: None,
                })
                .unwrap();
        });
    });

    group.finish();
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
