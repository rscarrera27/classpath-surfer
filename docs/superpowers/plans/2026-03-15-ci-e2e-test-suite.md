# CI-Compatible E2E Test Suite & Benchmarks Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make all E2E tests runnable in GitHub Actions CI, add diverse JVM library coverage, and add criterion benchmarks for refresh/search.

**Architecture:** Modify JDK resolution to use env vars with mise fallback, remove `#[ignore]` from E2E tests in favor of runtime skip, expand fixture dependencies, add new test cases for errors/output/status/show/library-diversity, add criterion benchmarks, and update CI workflow with a parallel `e2e` job.

**Tech Stack:** Rust 1.85, criterion 0.5, GitHub Actions (actions/setup-java@v4), Gradle fixture project

**Spec:** `docs/superpowers/specs/2026-03-15-ci-e2e-test-suite-design.md`

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `tests/e2e.rs` | Modify | JDK resolution rewrite, `#[ignore]` removal, new test cases |
| `tests/fixtures/gradle-project/app/build.gradle` | Modify | Add new dependencies |
| `.github/workflows/ci.yml` | Modify | Add `e2e` job, fix branch triggers |
| `Cargo.toml` | Modify | Add `criterion` dev-dependency, `[[bench]]` sections |
| `benches/search.rs` | Create | Search performance benchmarks |
| `benches/refresh.rs` | Create | Refresh performance benchmarks |

---

## Chunk 1: JDK Resolution & `#[ignore]` Removal

### Task 1: Rewrite `get_java_home` to support env var fallback

**Files:**
- Modify: `tests/e2e.rs:18-30`

- [ ] **Step 1: Rewrite `get_java_home` to return `Option<PathBuf>`**

Replace the current `get_java_home` function with:

```rust
/// Resolve JAVA_HOME for a given JDK major version.
///
/// Resolution order:
/// 1. `JAVA_{version}_HOME` env var (e.g. `JAVA_17_HOME`) — for CI
/// 2. `JAVA_HOME` env var — single-JDK environments
/// 3. `mise where java@temurin-{version}` — local dev fallback
/// 4. `None` — caller decides to skip
fn get_java_home(version: &str) -> Option<PathBuf> {
    // 1. JAVA_{version}_HOME
    if let Ok(home) = std::env::var(format!("JAVA_{version}_HOME")) {
        let path = PathBuf::from(home);
        if path.is_dir() {
            return Some(path);
        }
    }

    // 2. JAVA_HOME
    if let Ok(home) = std::env::var("JAVA_HOME") {
        let path = PathBuf::from(home);
        if path.is_dir() {
            return Some(path);
        }
    }

    // 3. mise fallback
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
```

- [ ] **Step 2: Add `require_jdk!` macro**

Add this macro right after `get_java_home`:

```rust
/// Skip a test if the required JDK version is not available.
macro_rules! require_jdk {
    ($version:expr) => {
        match get_java_home($version) {
            Some(home) => home,
            None => {
                eprintln!(
                    "JDK {} not available, skipping test '{}'",
                    $version,
                    stdext::function_name!()
                );
                return;
            }
        }
    };
}
```

Note: We can't use `stdext::function_name!()` without a dep. Simplify to:

```rust
macro_rules! require_jdk {
    ($version:expr) => {
        match get_java_home($version) {
            Some(home) => home,
            None => {
                eprintln!("JDK {} not available, skipping test", $version);
                return;
            }
        }
    };
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo test --test e2e --no-run`
Expected: Compiles successfully

- [ ] **Step 4: Commit**

```bash
git add tests/e2e.rs
git commit -m "refactor: rewrite get_java_home with env var fallback and require_jdk macro"
```

### Task 2: Remove `#[ignore]` and use `require_jdk!` in all JDK-dependent tests

**Files:**
- Modify: `tests/e2e.rs:114-267` (matrix tests), `tests/e2e.rs:451-684` (additional tests)

- [ ] **Step 1: Update `run_matrix_test` to accept `PathBuf` java_home**

Change line 114 to accept a `java_home: &Path` parameter:

```rust
fn run_matrix_test(java_home: &Path, jdk_version: &str, gradle_version: &str) {
    eprintln!(
        "=== Matrix test: JDK {jdk_version} ({}), Gradle {gradle_version} ===",
        java_home.display()
    );
    // ... rest stays the same, but remove the `let java_home = get_java_home(jdk_version);` line
```

- [ ] **Step 2: Remove `#[ignore]` from matrix tests and use `require_jdk!`**

Replace each matrix test. Example for the first one:

```rust
#[test]
fn test_jdk17_gradle7() {
    let java_home = require_jdk!("17");
    run_matrix_test(&java_home, "17", "7.6.4");
}
```

Apply the same pattern to all 5 matrix tests (`test_jdk17_gradle7`, `test_jdk17_gradle8_5`, `test_jdk17_gradle8_14`, `test_jdk21_gradle8_5`, `test_jdk21_gradle8_14`).

- [ ] **Step 3: Remove `#[ignore]` from single-combo tests and use `require_jdk!`**

Update `test_incremental_indexing`, `test_staleness_detection`, `test_show_no_source_fails_with_no_decompile`, `test_kotlin_metadata_extraction`.

Example for `test_incremental_indexing`:

```rust
#[test]
fn test_incremental_indexing() {
    let java_home = require_jdk!("21");
    let temp = tempfile::tempdir().unwrap();
    // ... rest unchanged, already uses java_home variable
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo test --test e2e --no-run`
Expected: Compiles successfully

- [ ] **Step 5: Commit**

```bash
git add tests/e2e.rs
git commit -m "refactor: remove #[ignore] from E2E tests, use require_jdk for graceful skip"
```

---

## Chunk 2: Fixture Dependencies & Library-Diversity Tests

### Task 3: Expand fixture project dependencies

**Files:**
- Modify: `tests/fixtures/gradle-project/app/build.gradle`

- [ ] **Step 1: Add new dependencies to app/build.gradle**

```gradle
plugins {
    id 'java'
}

dependencies {
    // Pure Java (existing)
    implementation 'com.google.guava:guava:33.4.0-jre'
    implementation 'com.google.code.gson:gson:2.11.0'

    // Kotlin/JVM (existing)
    implementation 'org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0'

    // Kotlin/JVM (new)
    implementation 'org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3'
    implementation 'io.ktor:ktor-client-core:3.0.3'

    // KMP -jvm artifacts (new)
    implementation 'org.jetbrains.kotlinx:kotlinx-datetime-jvm:0.6.1'
    implementation 'org.jetbrains.kotlinx:kotlinx-io-core-jvm:0.6.0'

    // Annotation processor (new)
    implementation 'com.google.dagger:dagger:2.52'

    // Large libraries (new)
    implementation 'org.springframework:spring-core:6.2.2'
    implementation 'com.squareup.okhttp3:okhttp:4.12.0'

    // Interface-only (new)
    implementation 'org.slf4j:slf4j-api:2.0.16'
    implementation 'jakarta.servlet:jakarta.servlet-api:6.1.0'
}
```

- [ ] **Step 2: Delete pre-computed index/manifest state from fixture**

The fixture has pre-computed `.classpath-surfer/` state that will be stale with new deps. Tests always copy the fixture and run their own init+refresh, so this cached state is unnecessary. Remove it to avoid confusion:

```bash
rm -rf tests/fixtures/gradle-project/.classpath-surfer/index/
rm -f tests/fixtures/gradle-project/.classpath-surfer/classpath-manifest.json
rm -f tests/fixtures/gradle-project/.classpath-surfer/indexed-manifest.json
rm -f tests/fixtures/gradle-project/.classpath-surfer/build-file-mtimes.json
rm -f tests/fixtures/gradle-project/.classpath-surfer/lockfile-hash
```

Keep `config.json` and `init-script.gradle` as they are template files.

- [ ] **Step 3: Commit**

```bash
git add tests/fixtures/gradle-project/
git commit -m "feat: expand fixture deps with Kotlin/JVM, KMP, Dagger, Spring, OkHttp, SLF4J, Jakarta"
```

### Task 4: Add library-diversity E2E tests

**Files:**
- Modify: `tests/e2e.rs` (append new tests at end)

- [ ] **Step 1: Add Kotlin/JVM library test**

```rust
#[test]
fn test_kotlin_jvm_symbols() {
    let java_home = require_jdk!("21");
    let temp = tempfile::tempdir().unwrap();
    let project_dir = copy_fixture_project(temp.path());
    set_gradle_version(&project_dir, "8.14");
    cli::init::run(&project_dir).unwrap();
    refresh_with_java_home(&project_dir, &java_home);

    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir).unwrap();

    // kotlinx-serialization: Json class
    let results = reader
        .search("Json", "class", false, false, 20, None)
        .unwrap();
    let json_class = results
        .iter()
        .find(|r| r.fqn.contains("kotlinx.serialization.json"));
    assert!(json_class.is_some(), "kotlinx.serialization.json.Json should be found");
    assert_eq!(
        json_class.unwrap().source_language.as_deref(),
        Some("kotlin"),
        "Json should be identified as Kotlin"
    );

    // kotlinx-serialization: Serializable annotation (transitive from core)
    let results = reader
        .search("Serializable", "class", false, false, 20, None)
        .unwrap();
    let serializable = results
        .iter()
        .find(|r| r.fqn.contains("kotlinx.serialization"));
    assert!(serializable.is_some(), "kotlinx.serialization.Serializable should be found (transitive)");

    // Ktor: HttpClient class
    let results = reader
        .search("HttpClient", "class", false, false, 20, None)
        .unwrap();
    let ktor = results
        .iter()
        .find(|r| r.fqn.contains("io.ktor.client"));
    assert!(ktor.is_some(), "io.ktor.client.HttpClient should be found");

    eprintln!("=== Kotlin/JVM symbols test passed ===");
}
```

- [ ] **Step 2: Add KMP JVM artifact test**

```rust
#[test]
fn test_kmp_jvm_symbols() {
    let java_home = require_jdk!("21");
    let temp = tempfile::tempdir().unwrap();
    let project_dir = copy_fixture_project(temp.path());
    set_gradle_version(&project_dir, "8.14");
    cli::init::run(&project_dir).unwrap();
    refresh_with_java_home(&project_dir, &java_home);

    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir).unwrap();

    // kotlinx-datetime: Instant
    let results = reader
        .search("Instant", "class", false, false, 20, None)
        .unwrap();
    let instant = results
        .iter()
        .find(|r| r.fqn.contains("kotlinx.datetime"));
    assert!(instant.is_some(), "kotlinx.datetime.Instant should be found");

    // kotlinx-datetime: Clock
    let results = reader
        .search("Clock", "class", false, false, 20, None)
        .unwrap();
    let clock = results
        .iter()
        .find(|r| r.fqn.contains("kotlinx.datetime"));
    assert!(clock.is_some(), "kotlinx.datetime.Clock should be found");

    // kotlinx-io: Buffer
    let results = reader
        .search("Buffer", "class", false, false, 20, None)
        .unwrap();
    let buffer = results.iter().find(|r| r.fqn.contains("kotlinx.io"));
    assert!(buffer.is_some(), "kotlinx.io.Buffer should be found");

    // kotlinx-io: Source
    let results = reader
        .search("Source", "class", false, false, 20, None)
        .unwrap();
    let source = results.iter().find(|r| r.fqn.contains("kotlinx.io"));
    assert!(source.is_some(), "kotlinx.io.Source should be found");

    eprintln!("=== KMP JVM symbols test passed ===");
}
```

- [ ] **Step 3: Add annotation processor test**

```rust
#[test]
fn test_annotation_processor_symbols() {
    let java_home = require_jdk!("21");
    let temp = tempfile::tempdir().unwrap();
    let project_dir = copy_fixture_project(temp.path());
    set_gradle_version(&project_dir, "8.14");
    cli::init::run(&project_dir).unwrap();
    refresh_with_java_home(&project_dir, &java_home);

    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir).unwrap();

    // Dagger: Component annotation
    let results = reader
        .search("Component", "class", false, false, 20, None)
        .unwrap();
    let dagger = results
        .iter()
        .find(|r| r.fqn == "dagger.Component");
    assert!(dagger.is_some(), "dagger.Component should be found");

    // Dagger: Module annotation
    let results = reader
        .search("Module", "class", false, false, 20, None)
        .unwrap();
    let module = results
        .iter()
        .find(|r| r.fqn == "dagger.Module");
    assert!(module.is_some(), "dagger.Module should be found");

    // Dagger: Provides annotation
    let results = reader
        .search("Provides", "class", false, false, 20, None)
        .unwrap();
    let provides = results
        .iter()
        .find(|r| r.fqn == "dagger.Provides");
    assert!(provides.is_some(), "dagger.Provides should be found");

    eprintln!("=== Annotation processor symbols test passed ===");
}
```

- [ ] **Step 4: Add large library test**

```rust
#[test]
fn test_large_library_symbols() {
    let java_home = require_jdk!("21");
    let temp = tempfile::tempdir().unwrap();
    let project_dir = copy_fixture_project(temp.path());
    set_gradle_version(&project_dir, "8.14");
    cli::init::run(&project_dir).unwrap();
    refresh_with_java_home(&project_dir, &java_home);

    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir).unwrap();

    // Spring Core: Environment
    let results = reader
        .search("Environment", "class", false, false, 20, None)
        .unwrap();
    let spring = results
        .iter()
        .find(|r| r.fqn.contains("springframework"));
    assert!(spring.is_some(), "org.springframework.core.env.Environment should be found");

    // OkHttp: OkHttpClient
    let results = reader
        .search("OkHttpClient", "class", false, false, 20, None)
        .unwrap();
    let okhttp = results
        .iter()
        .find(|r| r.fqn.contains("okhttp3"));
    assert!(okhttp.is_some(), "okhttp3.OkHttpClient should be found");

    // With many more deps, symbol count should be well above previous threshold
    let count = reader.count_symbols().unwrap();
    assert!(
        count > 1000,
        "Expected at least 1000 symbols with expanded deps, got {count}"
    );

    eprintln!("=== Large library symbols test passed ({count} symbols) ===");
}
```

- [ ] **Step 5: Add interface-only library test**

```rust
#[test]
fn test_interface_only_symbols() {
    let java_home = require_jdk!("21");
    let temp = tempfile::tempdir().unwrap();
    let project_dir = copy_fixture_project(temp.path());
    set_gradle_version(&project_dir, "8.14");
    cli::init::run(&project_dir).unwrap();
    refresh_with_java_home(&project_dir, &java_home);

    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir).unwrap();

    // SLF4J: Logger interface
    let results = reader
        .search("Logger", "class", false, false, 20, None)
        .unwrap();
    let slf4j = results
        .iter()
        .find(|r| r.fqn == "org.slf4j.Logger");
    assert!(slf4j.is_some(), "org.slf4j.Logger should be found");

    // SLF4J: LoggerFactory
    let results = reader
        .search("LoggerFactory", "class", false, false, 20, None)
        .unwrap();
    let factory = results
        .iter()
        .find(|r| r.fqn == "org.slf4j.LoggerFactory");
    assert!(factory.is_some(), "org.slf4j.LoggerFactory should be found");

    // Jakarta Servlet: HttpServlet
    let results = reader
        .search("HttpServlet", "class", false, false, 20, None)
        .unwrap();
    let servlet = results
        .iter()
        .find(|r| r.fqn.contains("jakarta.servlet"));
    assert!(servlet.is_some(), "jakarta.servlet.http.HttpServlet should be found");

    // Jakarta Servlet: Filter interface
    let results = reader
        .search("Filter", "class", false, false, 20, None)
        .unwrap();
    let filter = results
        .iter()
        .find(|r| r.fqn == "jakarta.servlet.Filter");
    assert!(filter.is_some(), "jakarta.servlet.Filter should be found");

    eprintln!("=== Interface-only symbols test passed ===");
}
```

- [ ] **Step 6: Verify all new tests compile**

Run: `cargo test --test e2e --no-run`
Expected: Compiles successfully

- [ ] **Step 7: Commit**

```bash
git add tests/e2e.rs
git commit -m "feat: add library-diversity E2E tests for Kotlin/JVM, KMP, Dagger, Spring, SLF4J, Jakarta"
```

---

## Chunk 3: Error, Output Mode, Status/Clean, and Source Tests

### Task 5: Add error case tests

**Files:**
- Modify: `tests/e2e.rs`

- [ ] **Step 1: Add `test_search_without_index`**

```rust
#[test]
fn test_search_without_index() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    std::fs::create_dir_all(&project_dir).unwrap();

    // No init, no index — search should fail
    let result = cli::search::run(&project_dir, "Foo", "any", false, false, 10, None);
    assert!(result.is_err(), "search without index should fail");

    let err = result.unwrap_err();
    let cli_err = err.downcast_ref::<classpath_surfer::error::CliError>().unwrap();
    assert_eq!(cli_err.exit_code, 3, "exit code should be 3 (resource not found)");
    assert_eq!(cli_err.error_code, "INDEX_NOT_FOUND");
}
```

- [ ] **Step 2: Add `test_refresh_invalid_project`**

```rust
#[test]
fn test_refresh_invalid_project() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("nonexistent");
    // Don't create the directory — it shouldn't exist

    let configs = vec!["compileClasspath".to_string()];
    let result = cli::refresh::run_with_java_home(&project_dir, &configs, true, None);
    assert!(result.is_err(), "refresh on non-existent project should fail");
}
```

- [ ] **Step 3: Add `test_search_no_results`**

```rust
#[test]
fn test_search_no_results() {
    let java_home = require_jdk!("21");
    let temp = tempfile::tempdir().unwrap();
    let project_dir = copy_fixture_project(temp.path());
    set_gradle_version(&project_dir, "8.14");
    cli::init::run(&project_dir).unwrap();
    refresh_with_java_home(&project_dir, &java_home);

    // Search for something that definitely doesn't exist
    let output = cli::search::run(
        &project_dir,
        "XyzNonExistentClassName12345",
        "any",
        false,
        false,
        10,
        None,
    )
    .expect("search with no results should succeed (not error)");

    assert!(output.results.is_empty(), "should return empty results, not an error");
}
```

- [ ] **Step 4: Verify compilation**

Run: `cargo test --test e2e --no-run`
Expected: Compiles. Note: `test_search_without_index` imports `classpath_surfer::error::CliError` — ensure this is in scope at the top of the file.

- [ ] **Step 5: Commit**

```bash
git add tests/e2e.rs
git commit -m "feat: add error case E2E tests (search without index, invalid project, no results)"
```

### Task 6: Add agentic output mode tests

**Files:**
- Modify: `tests/e2e.rs`

These tests call the CLI handlers directly and verify the output structure matches what `--agentic` mode would serialize. Since the handlers return typed structs, we serialize them to JSON and parse back to verify field presence.

- [ ] **Step 1: Add `test_agentic_search_output`**

```rust
#[test]
fn test_agentic_search_output() {
    let java_home = require_jdk!("21");
    let temp = tempfile::tempdir().unwrap();
    let project_dir = copy_fixture_project(temp.path());
    set_gradle_version(&project_dir, "8.14");
    cli::init::run(&project_dir).unwrap();
    refresh_with_java_home(&project_dir, &java_home);

    let output = cli::search::run(&project_dir, "ImmutableList", "class", false, false, 10, None)
        .expect("search should succeed");

    // Serialize to JSON and verify structure
    let json = serde_json::to_value(&output).unwrap();
    assert!(json.get("query").is_some(), "JSON should have 'query' field");
    assert!(json.get("results").is_some(), "JSON should have 'results' field");

    let results = json["results"].as_array().unwrap();
    assert!(!results.is_empty(), "results should not be empty");

    let first = &results[0];
    assert!(first.get("fqn").is_some(), "result should have 'fqn'");
    assert!(first.get("symbol_kind").is_some(), "result should have 'symbol_kind'");
    assert!(first.get("gav").is_some(), "result should have 'gav'");
    assert!(first.get("simple_name").is_some(), "result should have 'simple_name'");
    assert!(first.get("signature_display").is_some(), "result should have 'signature_display'");
    assert!(first.get("access_flags").is_some(), "result should have 'access_flags'");
    assert!(first.get("has_source").is_some(), "result should have 'has_source'");
    assert!(first.get("score").is_some(), "result should have 'score'");

    eprintln!("=== Agentic search output test passed ===");
}
```

- [ ] **Step 2: Add `test_agentic_error_output`**

```rust
#[test]
fn test_agentic_error_output() {
    use classpath_surfer::error::CliError;

    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    std::fs::create_dir_all(&project_dir).unwrap();

    let result = cli::search::run(&project_dir, "Foo", "any", false, false, 10, None);
    let err = result.unwrap_err();
    let cli_err = err.downcast_ref::<CliError>().unwrap();

    // Serialize the error like --agentic mode does
    let json = serde_json::json!({
        "success": false,
        "error_code": cli_err.error_code,
        "error": &cli_err.message,
        "retryable": cli_err.retryable,
    });

    assert_eq!(json["success"], false);
    assert!(json["error_code"].is_string(), "error_code should be a string");
    assert!(json["error"].is_string(), "error should be a string");
    assert!(json["retryable"].is_boolean(), "retryable should be a boolean");

    eprintln!("=== Agentic error output test passed ===");
}
```

- [ ] **Step 3: Add `test_agentic_exit_codes`**

```rust
#[test]
fn test_agentic_exit_codes() {
    use classpath_surfer::error::CliError;

    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    std::fs::create_dir_all(&project_dir).unwrap();

    // INDEX_NOT_FOUND → exit code 3
    let result = cli::search::run(&project_dir, "Foo", "any", false, false, 10, None);
    let err = result.unwrap_err();
    let cli_err = err.downcast_ref::<CliError>().unwrap();
    assert_eq!(cli_err.exit_code, 3, "INDEX_NOT_FOUND should have exit code 3");
    assert_eq!(cli_err.error_code, "INDEX_NOT_FOUND");
}
```

- [ ] **Step 4: Verify compilation**

Run: `cargo test --test e2e --no-run`
Expected: Compiles. `CliError` has public fields — tests use direct field access (e.g. `cli_err.exit_code`).

- [ ] **Step 5: Commit**

```bash
git add tests/e2e.rs
git commit -m "feat: add agentic output mode E2E tests (JSON structure, error format, exit codes)"
```

### Task 7: Add status/clean E2E tests

**Files:**
- Modify: `tests/e2e.rs`

- [ ] **Step 1: Add `test_status_after_refresh`**

```rust
#[test]
fn test_status_after_refresh() {
    let java_home = require_jdk!("21");
    let temp = tempfile::tempdir().unwrap();
    let project_dir = copy_fixture_project(temp.path());
    set_gradle_version(&project_dir, "8.14");
    cli::init::run(&project_dir).unwrap();
    refresh_with_java_home(&project_dir, &java_home);

    let status = cli::status::run(&project_dir).expect("status should succeed");

    assert!(status.initialized, "should be initialized");
    assert!(status.has_index, "should have index after refresh");
    assert!(status.dependency_count > 0, "should have dependencies");
    assert!(
        status.indexed_symbols.is_some() && status.indexed_symbols.unwrap() > 0,
        "should have indexed symbols"
    );
    assert!(!status.is_stale, "should not be stale right after refresh");
    assert!(status.index_size.is_some(), "should report index size");

    // With expanded deps, we should have some without source JARs too
    assert!(status.with_source_jars > 0, "should have deps with source JARs");

    eprintln!("=== Status after refresh test passed ===");
}
```

- [ ] **Step 2: Add `test_clean_then_status`**

```rust
#[test]
fn test_clean_then_status() {
    let java_home = require_jdk!("21");
    let temp = tempfile::tempdir().unwrap();
    let project_dir = copy_fixture_project(temp.path());
    set_gradle_version(&project_dir, "8.14");
    cli::init::run(&project_dir).unwrap();
    refresh_with_java_home(&project_dir, &java_home);

    // Clean
    cli::clean::run(&project_dir).expect("clean should succeed");

    // Status after clean
    let status = cli::status::run(&project_dir).expect("status after clean should succeed");
    assert!(status.initialized, "should still be initialized (config remains)");
    assert!(!status.has_index, "should not have index after clean");
    assert!(status.indexed_symbols.is_none(), "indexed_symbols should be None without index");

    eprintln!("=== Clean then status test passed ===");
}
```

- [ ] **Step 3: Add `test_clean_idempotent_e2e`**

```rust
#[test]
fn test_clean_idempotent_e2e() {
    let java_home = require_jdk!("21");
    let temp = tempfile::tempdir().unwrap();
    let project_dir = copy_fixture_project(temp.path());
    set_gradle_version(&project_dir, "8.14");
    cli::init::run(&project_dir).unwrap();
    refresh_with_java_home(&project_dir, &java_home);

    // Clean twice — should not error
    cli::clean::run(&project_dir).expect("first clean should succeed");
    cli::clean::run(&project_dir).expect("second clean should succeed (idempotent)");

    eprintln!("=== Clean idempotent E2E test passed ===");
}
```

- [ ] **Step 4: Verify compilation**

Run: `cargo test --test e2e --no-run`
Expected: Compiles successfully

- [ ] **Step 5: Commit**

```bash
git add tests/e2e.rs
git commit -m "feat: add status/clean E2E tests (status after refresh, clean then status, idempotent)"
```

### Task 8: Add source resolution tests

**Files:**
- Modify: `tests/e2e.rs`

- [ ] **Step 1: Add `test_show_with_source_jar`**

```rust
#[test]
fn test_show_with_source_jar() {
    let java_home = require_jdk!("21");
    let temp = tempfile::tempdir().unwrap();
    let project_dir = copy_fixture_project(temp.path());
    set_gradle_version(&project_dir, "8.14");
    cli::init::run(&project_dir).unwrap();
    refresh_with_java_home(&project_dir, &java_home);

    let output = cli::show::run(
        &project_dir,
        "com.google.gson.Gson",
        "cfr",
        None,
        true, // no_decompile — we want original source only
    )
    .expect("show should succeed for Gson (has source JAR)");

    assert_eq!(output.fqn, "com.google.gson.Gson");
    assert!(!output.gav.is_empty(), "gav should not be empty");
    assert!(
        output.primary.content.contains("class Gson"),
        "source should contain 'class Gson'"
    );
    assert_eq!(output.primary.language, "java");
    assert_eq!(output.primary.source_type, "original");

    eprintln!("=== Show with source JAR test passed ===");
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo test --test e2e --no-run`
Expected: Compiles successfully

- [ ] **Step 3: Commit**

```bash
git add tests/e2e.rs
git commit -m "feat: add source resolution E2E test (show with source JAR)"
```

---

## Chunk 4: CI Workflow

### Task 9: Update GitHub Actions CI workflow

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Fix branch trigger and add `e2e` job**

Replace the full `ci.yml` with:

```yaml
name: CI

on:
  push:
    branches: [master]
  pull_request:
    branches: [master]

env:
  CARGO_TERM_COLOR: always

jobs:
  check:
    name: Check (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.85
        with:
          components: clippy, rustfmt
      - uses: Swatinem/rust-cache@v2

      - name: Check formatting
        run: cargo fmt -- --check

      - name: Clippy
        run: cargo clippy -- -D warnings

      - name: Run tests
        run: cargo test

      - name: Check docs build
        run: RUSTDOCFLAGS="-D warnings" cargo doc --no-deps

  e2e:
    name: E2E (JDK ${{ matrix.jdk }}, Gradle ${{ matrix.gradle }}, ${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            jdk: 17
            gradle: "7.6.4"
          - os: ubuntu-latest
            jdk: 21
            gradle: "8.14"
          - os: macos-latest
            jdk: 17
            gradle: "7.6.4"
          - os: macos-latest
            jdk: 21
            gradle: "8.14"
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@1.85

      - uses: Swatinem/rust-cache@v2

      - uses: actions/setup-java@v4
        with:
          distribution: temurin
          java-version: ${{ matrix.jdk }}

      - name: Set JAVA version-specific HOME
        run: echo "JAVA_${{ matrix.jdk }}_HOME=$JAVA_HOME" >> $GITHUB_ENV

      - name: Install protoc (Ubuntu)
        if: runner.os == 'Linux'
        run: sudo apt-get install -y protobuf-compiler

      - name: Install protoc (macOS)
        if: runner.os == 'macOS'
        run: brew install protobuf

      - name: Run E2E tests
        run: cargo test --test e2e -- --test-threads=1
```

Note: `--test-threads=1` to avoid concurrent Gradle executions fighting over the daemon.

- [ ] **Step 2: Verify YAML syntax**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add E2E job with JDK/Gradle matrix, fix branch trigger to master"
```

---

## Chunk 5: Criterion Benchmarks

### Task 10: Add criterion dependency and bench config

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add criterion to dev-dependencies and bench targets**

Add to `Cargo.toml`:

```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }
tempfile = "3"
serde_json = "1"

[[bench]]
name = "search"
harness = false

[[bench]]
name = "refresh"
harness = false
```

Note: `tempfile` and `serde_json` are already in `[dependencies]`, but they're needed for benchmarks too. Since they're already deps, the bench code can use them directly — no need to add to `[dev-dependencies]` separately. Only add `criterion`:

```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "search"
harness = false

[[bench]]
name = "refresh"
harness = false
```

- [ ] **Step 2: Commit**

```bash
git add Cargo.toml
git commit -m "build: add criterion benchmark dependency and bench targets"
```

### Task 11: Create search benchmark

**Files:**
- Create: `benches/search.rs`

- [ ] **Step 1: Write search benchmark**

```rust
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};

use classpath_surfer::cli;
use classpath_surfer::index::reader::IndexReader;

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
            reader
                .search("ImmutableList", "any", false, false, 20, None)
                .unwrap()
        });
    });

    group.bench_function("fqn", |b| {
        b.iter(|| {
            reader
                .search(
                    "com.google.common.collect.ImmutableList",
                    "any",
                    true,
                    false,
                    20,
                    None,
                )
                .unwrap()
        });
    });

    group.bench_function("regex", |b| {
        b.iter(|| {
            reader
                .search("Immutable.*", "any", false, true, 20, None)
                .unwrap()
        });
    });

    group.bench_function("type_filter", |b| {
        b.iter(|| {
            reader
                .search("ImmutableList", "class", false, false, 20, None)
                .unwrap()
        });
    });

    group.bench_function("dependency_filter", |b| {
        b.iter(|| {
            reader
                .search("ImmutableList", "any", false, false, 20, Some("com.google.guava:guava:33.4.0-jre"))
                .unwrap()
        });
    });

    group.finish();
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo bench --bench search --no-run`
Expected: Compiles successfully

- [ ] **Step 3: Commit**

```bash
git add benches/search.rs
git commit -m "feat: add criterion search benchmarks (simple, fqn, regex, type/dep filter)"
```

### Task 12: Create refresh benchmark

**Files:**
- Create: `benches/refresh.rs`

- [ ] **Step 1: Write refresh benchmark**

```rust
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
        cli::refresh::run_with_java_home(
            &project_dir,
            &configs,
            true,
            Some(&java_home),
        )
        .unwrap();

        b.iter(|| {
            cli::refresh::run_with_java_home(
                &project_dir,
                &configs,
                false,
                Some(&java_home),
            )
            .unwrap();
        });
    });

    group.finish();
}

criterion_group!(benches, bench_refresh);
criterion_main!(benches);
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo bench --bench refresh --no-run`
Expected: Compiles successfully

- [ ] **Step 3: Commit**

```bash
git add benches/refresh.rs
git commit -m "feat: add criterion refresh benchmarks (full, incremental, noop)"
```

---

## Chunk 6: Verification & Cleanup

### Task 13: Run full test suite and fix issues

- [ ] **Step 1: Run cargo clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings

- [ ] **Step 2: Run cargo fmt**

Run: `cargo fmt -- --check`
Expected: No formatting issues. If issues found, run `cargo fmt` and commit.

- [ ] **Step 3: Run non-JDK tests**

Run: `cargo test --test e2e test_manifest_diff test_manifest_merge_dedup test_init_idempotent test_clean_command test_search_without_index test_agentic_error_output test_agentic_exit_codes`
Expected: All pass (these don't need JDK)

- [ ] **Step 4: Run JDK-dependent tests locally (if JDK available)**

Run: `cargo test --test e2e -- --test-threads=1`
Expected: JDK-dependent tests either pass (if JDK available via mise) or skip gracefully

- [ ] **Step 5: Run benchmarks compile check**

Run: `cargo bench --no-run`
Expected: Both bench targets compile

- [ ] **Step 6: Final commit if any fixes were needed**

```bash
git add -A
git commit -m "fix: address clippy/fmt issues from E2E test suite additions"
```
