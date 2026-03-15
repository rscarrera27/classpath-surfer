mod common;

use classpath_surfer::cli;
use classpath_surfer::index::reader::IndexReader;
use classpath_surfer::model::SearchQuery;
use common::require_jdk;

// ---------------------------------------------------------------------------
// Matrix test runner
// ---------------------------------------------------------------------------

fn run_matrix_test(java_home: &std::path::Path, jdk_version: &str, gradle_version: &str) {
    eprintln!(
        "=== Matrix test: JDK {jdk_version} ({}), Gradle {gradle_version} ===",
        java_home.display()
    );

    let temp = tempfile::tempdir().unwrap();
    let project_dir = common::copy_fixture_project(temp.path());

    // Set the desired Gradle version
    common::set_gradle_version(&project_dir, gradle_version);

    // Init
    cli::init::run(&project_dir).expect("init should succeed");

    // Refresh with specific JAVA_HOME
    common::refresh_with_java_home(&project_dir, java_home);

    // Open the index and verify search results
    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir).expect("index should be readable");

    // ImmutableList (guava, app module)
    let (results, _count, _) = reader
        .search(&SearchQuery {
            query: Some("ImmutableList"),
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 10,
            dependency: None,
            access_levels: None,
            offset: 0,
            scope: None,
        })
        .expect("search should succeed");
    assert!(
        !results.is_empty(),
        "ImmutableList should be found (guava dependency in :app)"
    );

    // StringUtils (commons-lang3, lib module)
    let (results, _count, _) = reader
        .search(&SearchQuery {
            query: Some("StringUtils"),
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 10,
            dependency: None,
            access_levels: None,
            offset: 0,
            scope: None,
        })
        .expect("search should succeed");
    assert!(
        !results.is_empty(),
        "StringUtils should be found (commons-lang3 dependency in :lib)"
    );

    // Gson (gson, app module)
    let (results, _count, _) = reader
        .search(&SearchQuery {
            query: Some("Gson"),
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 10,
            dependency: None,
            access_levels: None,
            offset: 0,
            scope: None,
        })
        .expect("search should succeed");
    assert!(
        !results.is_empty(),
        "Gson should be found (gson dependency in :app)"
    );

    // Verify symbol count is reasonable
    let count = reader.count_symbols().unwrap();
    assert!(
        count > 100,
        "Expected at least 100 symbols from guava+gson+commons-lang3, got {count}"
    );

    // Verify manifest has source JAR info
    let manifest = common::read_manifest(&project_dir);
    let deps = manifest.all_dependencies();
    let has_source = deps.iter().any(|d| d.source_jar_path.is_some());
    assert!(
        has_source,
        "At least one dependency should have a source JAR"
    );

    // --- Kotlin metadata verification ---

    // 1. Kotlin class should be searchable
    let (results, _count, _) = reader
        .search(&SearchQuery {
            query: Some("CoroutineScope"),
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 10,
            dependency: None,
            access_levels: None,
            offset: 0,
            scope: None,
        })
        .expect("search should succeed");
    assert!(
        !results.is_empty(),
        "CoroutineScope should be found (kotlinx-coroutines dependency in :app)"
    );

    // 2. source_language should be "kotlin"
    let scope_result = &results[0];
    assert_eq!(
        scope_result
            .source_language
            .map(|l| l.to_string())
            .as_deref(),
        Some("kotlin"),
        "CoroutineScope source_language should be 'kotlin'"
    );

    // 3. kotlin_signature_display should be populated
    assert!(
        scope_result.signature.kotlin.is_some(),
        "CoroutineScope should have kotlin signature"
    );

    // 4. Kotlin methods should be searchable with signatures
    let (results, _count, _) = reader
        .search(&SearchQuery {
            query: Some("cancel"),
            symbol_type: "method",
            fqn_mode: false,
            regex_mode: false,
            limit: 100,
            dependency: None,
            access_levels: None,
            offset: 0,
            scope: None,
        })
        .expect("search should succeed");
    let kotlin_methods: Vec<_> = results
        .iter()
        .filter(|r| {
            r.source_language.map(|l| l.to_string()).as_deref() == Some("kotlin")
                && r.fqn.contains("kotlinx.coroutines")
        })
        .collect();
    assert!(
        !kotlin_methods.is_empty(),
        "Should find Kotlin methods from kotlinx-coroutines"
    );

    // 5. Kotlin method signatures should contain 'fun' keyword
    let has_fun_keyword = kotlin_methods
        .iter()
        .any(|r| r.signature.kotlin.as_deref().unwrap_or("").contains("fun"));
    assert!(
        has_fun_keyword,
        "Kotlin method signatures should contain 'fun' keyword"
    );

    eprintln!(
        "=== Matrix test passed: JDK {jdk_version}, Gradle {gradle_version} ({count} symbols) ==="
    );
}

// ---------------------------------------------------------------------------
// Matrix tests
// ---------------------------------------------------------------------------

#[test]
fn jdk17_gradle7() {
    let java_home = require_jdk!("17");
    run_matrix_test(&java_home, "17", "7.6.4");
}

#[test]
fn jdk17_gradle8_5() {
    let java_home = require_jdk!("17");
    run_matrix_test(&java_home, "17", "8.5");
}

#[test]
fn jdk17_gradle8_14() {
    let java_home = require_jdk!("17");
    run_matrix_test(&java_home, "17", "8.14");
}

#[test]
fn jdk21_gradle8_5() {
    let java_home = require_jdk!("21");
    run_matrix_test(&java_home, "21", "8.5");
}

#[test]
fn jdk21_gradle8_14() {
    let java_home = require_jdk!("21");
    run_matrix_test(&java_home, "21", "8.14");
}
