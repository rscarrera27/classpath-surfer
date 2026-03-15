mod common;

use classpath_surfer::cli;
use classpath_surfer::index::reader::IndexReader;
use classpath_surfer::model::SearchQuery;
use classpath_surfer::staleness;
use common::require_jdk;

// ---------------------------------------------------------------------------
// Tests that modify the project (each needs its own fresh copy)
// ---------------------------------------------------------------------------

#[test]
fn incremental_indexing() {
    let java_home = require_jdk!("21");
    let (_temp, project_dir) = common::fresh_project("8.14");

    // 1. Full refresh
    common::refresh_with_java_home(&project_dir, &java_home);

    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir).unwrap();
    let initial_count = reader.count_symbols().unwrap();
    assert!(initial_count > 0);

    // Verify Gson is found
    let (results, _count) = reader
        .search(&SearchQuery {
            query: "Gson",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 10,
            dependency: None,
            access_levels: None,
        })
        .unwrap();
    assert!(!results.is_empty(), "Gson should be found before removal");
    drop(reader);

    // 2. Remove gson from app/build.gradle
    let app_build = project_dir.join("app/build.gradle");
    let content = std::fs::read_to_string(&app_build).unwrap();
    let new_content = content
        .lines()
        .filter(|l| !l.contains("gson"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&app_build, new_content).unwrap();

    // 3. Incremental refresh (not full)
    let configs = common::default_configurations();
    cli::refresh::run_with_java_home(&project_dir, &configs, false, Some(&java_home))
        .expect("incremental refresh should succeed");

    // 4. Verify Gson is gone, ImmutableList still present
    let reader = IndexReader::open(&index_dir).unwrap();

    let (results, _count) = reader
        .search(&SearchQuery {
            query: "Gson",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 10,
            dependency: None,
            access_levels: None,
        })
        .unwrap();
    assert!(
        results.is_empty(),
        "Gson should NOT be found after removing dependency"
    );

    let (results, _count) = reader
        .search(&SearchQuery {
            query: "ImmutableList",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 10,
            dependency: None,
            access_levels: None,
        })
        .unwrap();
    assert!(
        !results.is_empty(),
        "ImmutableList should still be found after incremental update"
    );

    let new_count = reader.count_symbols().unwrap();
    assert!(
        new_count < initial_count,
        "Symbol count should decrease after removing gson ({new_count} >= {initial_count})"
    );
}

#[test]
fn staleness_detection() {
    let java_home = require_jdk!("21");
    let (_temp, project_dir) = common::fresh_project("8.14");

    // After init (which includes refresh via fresh_project), check staleness
    // Need a full refresh first to establish staleness baseline
    common::refresh_with_java_home(&project_dir, &java_home);

    assert!(
        !staleness::is_stale(&project_dir).unwrap(),
        "Should NOT be stale right after refresh"
    );

    // Modify build.gradle -> stale
    let build_file = project_dir.join("app/build.gradle");
    let content = std::fs::read_to_string(&build_file).unwrap();
    // Touch the file with a small delay to ensure mtime changes
    std::thread::sleep(std::time::Duration::from_secs(1));
    std::fs::write(&build_file, format!("{content}\n// modified")).unwrap();

    assert!(
        staleness::is_stale(&project_dir).unwrap(),
        "Should be stale after modifying build.gradle"
    );

    // Refresh again -> not stale
    common::refresh_with_java_home(&project_dir, &java_home);
    assert!(
        !staleness::is_stale(&project_dir).unwrap(),
        "Should NOT be stale after second refresh"
    );
}

#[test]
fn clean_then_status() {
    let java_home = require_jdk!("21");
    let (_temp, project_dir) = common::fresh_project("8.14");
    common::refresh_with_java_home(&project_dir, &java_home);

    cli::clean::run(&project_dir).expect("clean should succeed");

    let status = cli::status::run(&project_dir).expect("status after clean should succeed");
    assert!(status.initialized);
    assert!(!status.has_index);
    assert!(status.indexed_symbols.is_none());
}

#[test]
fn clean_idempotent() {
    let java_home = require_jdk!("21");
    let (_temp, project_dir) = common::fresh_project("8.14");
    common::refresh_with_java_home(&project_dir, &java_home);

    cli::clean::run(&project_dir).expect("first clean should succeed");
    cli::clean::run(&project_dir).expect("second clean should succeed (idempotent)");
}
