use std::path::PathBuf;

use classpath_surfer::cli;
use classpath_surfer::error::CliError;
use classpath_surfer::model::SearchQuery;

// ---------------------------------------------------------------------------
// Pure logic tests (no JDK/Gradle required)
// ---------------------------------------------------------------------------

#[test]
fn manifest_diff() {
    use classpath_surfer::manifest::diff::compute_diff;
    use classpath_surfer::manifest::{
        ClasspathManifest, ConfigurationManifest, DependencyInfo, ModuleManifest,
    };

    let make_dep = |group: &str, artifact: &str, version: &str| DependencyInfo {
        group: group.to_string(),
        artifact: artifact.to_string(),
        version: version.to_string(),
        jar_path: PathBuf::from("/fake.jar"),
        source_jar_path: None,
        scope: "compile".to_string(),
    };

    let manifest_a = ClasspathManifest {
        gradle_version: "8.14".to_string(),
        extraction_timestamp: "2024-01-01".to_string(),
        modules: vec![ModuleManifest {
            module_path: ":app".to_string(),
            configurations: vec![ConfigurationManifest {
                name: "compileClasspath".to_string(),
                dependencies: vec![
                    make_dep("com.google.guava", "guava", "33.4.0-jre"),
                    make_dep("com.google.code.gson", "gson", "2.11.0"),
                ],
            }],
        }],
    };

    let manifest_b = ClasspathManifest {
        gradle_version: "8.14".to_string(),
        extraction_timestamp: "2024-01-02".to_string(),
        modules: vec![ModuleManifest {
            module_path: ":app".to_string(),
            configurations: vec![ConfigurationManifest {
                name: "compileClasspath".to_string(),
                dependencies: vec![
                    make_dep("com.google.guava", "guava", "33.4.0-jre"),
                    make_dep("org.apache.commons", "commons-lang3", "3.17.0"),
                ],
            }],
        }],
    };

    let diff = compute_diff(&manifest_b, &manifest_a);
    assert!(
        diff.added
            .contains("org.apache.commons:commons-lang3:3.17.0")
    );
    assert!(diff.removed.contains("com.google.code.gson:gson:2.11.0"));
    assert!(diff.unchanged.contains("com.google.guava:guava:33.4.0-jre"));
}

#[test]
fn manifest_merge_dedup() {
    use classpath_surfer::manifest::merge::deduplicate;
    use classpath_surfer::manifest::{
        ClasspathManifest, ConfigurationManifest, DependencyInfo, ModuleManifest,
    };

    let dep = DependencyInfo {
        group: "com.google.guava".to_string(),
        artifact: "guava".to_string(),
        version: "33.4.0-jre".to_string(),
        jar_path: PathBuf::from("/fake.jar"),
        source_jar_path: None,
        scope: "compile".to_string(),
    };

    let manifest = ClasspathManifest {
        gradle_version: "8.14".to_string(),
        extraction_timestamp: "2024-01-01".to_string(),
        modules: vec![
            ModuleManifest {
                module_path: ":app".to_string(),
                configurations: vec![ConfigurationManifest {
                    name: "compileClasspath".to_string(),
                    dependencies: vec![dep.clone()],
                }],
            },
            ModuleManifest {
                module_path: ":lib".to_string(),
                configurations: vec![ConfigurationManifest {
                    name: "compileClasspath".to_string(),
                    dependencies: vec![dep.clone()],
                }],
            },
        ],
    };

    let unique = deduplicate(&manifest);
    assert_eq!(
        unique.len(),
        1,
        "Same GAV appearing in two modules should be deduplicated"
    );
    assert_eq!(unique[0].gav(), "com.google.guava:guava:33.4.0-jre");
}

#[test]
fn init_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    std::fs::create_dir_all(&project_dir).unwrap();

    // Write a minimal settings.gradle so it looks like a project
    std::fs::write(
        project_dir.join("settings.gradle"),
        "rootProject.name = 'test'\n",
    )
    .unwrap();

    // First init
    cli::init::run(&project_dir).expect("first init should succeed");
    let config1 =
        std::fs::read_to_string(project_dir.join(".classpath-surfer/config.json")).unwrap();

    // Second init (idempotent)
    cli::init::run(&project_dir).expect("second init should succeed");
    let config2 =
        std::fs::read_to_string(project_dir.join(".classpath-surfer/config.json")).unwrap();

    assert_eq!(config1, config2, "Init should be idempotent");

    // .gitignore should only have one entry
    let gitignore = std::fs::read_to_string(project_dir.join(".gitignore")).unwrap();
    let entry_count = gitignore
        .lines()
        .filter(|l| l.trim() == ".classpath-surfer/")
        .count();
    assert_eq!(
        entry_count, 1,
        ".gitignore should not have duplicate entries"
    );
}

#[test]
fn clean_command() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    let surfer_dir = project_dir.join(".classpath-surfer");
    std::fs::create_dir_all(surfer_dir.join("index")).unwrap();

    // Create fake files that clean should remove
    std::fs::write(surfer_dir.join("indexed-manifest.json"), "{}").unwrap();
    std::fs::write(surfer_dir.join("lockfile-hash"), "abc123").unwrap();
    std::fs::write(surfer_dir.join("build-file-mtimes.json"), "{}").unwrap();
    std::fs::write(surfer_dir.join("index/meta.json"), "{}").unwrap();

    cli::clean::run(&project_dir).expect("clean should succeed");

    assert!(
        !surfer_dir.join("index").exists(),
        "index dir should be removed"
    );
    assert!(
        !surfer_dir.join("indexed-manifest.json").exists(),
        "indexed manifest should be removed"
    );
    assert!(
        !surfer_dir.join("lockfile-hash").exists(),
        "lockfile hash should be removed"
    );
    assert!(
        !surfer_dir.join("build-file-mtimes.json").exists(),
        "build file mtimes should be removed"
    );

    // surfer_dir itself should still exist
    assert!(surfer_dir.exists(), ".classpath-surfer dir should remain");
}

// ---------------------------------------------------------------------------
// GAV pattern matching
// ---------------------------------------------------------------------------

#[test]
fn gav_pattern_exact_match() {
    assert!(cli::matches_gav_pattern(
        "com.google.guava:guava:33.0-jre",
        "com.google.guava:guava:33.0-jre"
    ));
    assert!(!cli::matches_gav_pattern(
        "com.google.guava:guava:33.0-jre",
        "com.google.guava:guava:34.0-jre"
    ));
}

#[test]
fn gav_pattern_wildcard_version() {
    assert!(cli::matches_gav_pattern(
        "com.google.guava:guava:33.0-jre",
        "com.google.guava:guava:*"
    ));
    assert!(!cli::matches_gav_pattern(
        "io.netty:netty-all:4.1",
        "com.google.guava:guava:*"
    ));
}

#[test]
fn gav_pattern_wildcard_group() {
    assert!(cli::matches_gav_pattern(
        "com.google.guava:guava:33.0-jre",
        "com.google.*:*"
    ));
    assert!(cli::matches_gav_pattern(
        "com.google.code.gson:gson:2.11",
        "com.google.*:*"
    ));
    assert!(!cli::matches_gav_pattern(
        "io.netty:netty-all:4.1",
        "com.google.*:*"
    ));
}

#[test]
fn gav_pattern_wildcard_artifact() {
    assert!(cli::matches_gav_pattern(
        "io.netty:netty-all:4.1",
        "*:netty-*:*"
    ));
    assert!(!cli::matches_gav_pattern(
        "com.google.guava:guava:33.0",
        "*:netty-*:*"
    ));
}

#[test]
fn gav_pattern_star_only() {
    assert!(cli::matches_gav_pattern(
        "com.google.guava:guava:33.0-jre",
        "*"
    ));
}

// ---------------------------------------------------------------------------
// Error case tests (no JDK required)
// ---------------------------------------------------------------------------

#[test]
fn search_without_index() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    std::fs::create_dir_all(&project_dir).unwrap();

    let result = cli::search::run(
        &project_dir,
        &SearchQuery {
            query: Some("Foo"),
            symbol_types: &[],
            fqn_mode: false,
            regex_mode: false,
            limit: 10,
            dependency: None,
            access_levels: &[],
            offset: 0,
            scope: None,
        },
    );
    assert!(result.is_err(), "search without index should fail");

    let err = result.unwrap_err();
    let cli_err = err.downcast_ref::<CliError>().unwrap();
    assert_eq!(cli_err.exit_code, 3);
    assert_eq!(cli_err.error_code, "INDEX_NOT_FOUND");
}

#[test]
fn refresh_invalid_project() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("nonexistent");

    let configs = vec!["compileClasspath".to_string()];
    let result = cli::refresh::run_with_java_home(&project_dir, &configs, true, None, 300);
    assert!(
        result.is_err(),
        "refresh on non-existent project should fail"
    );
}

#[test]
fn agentic_error_output() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    std::fs::create_dir_all(&project_dir).unwrap();

    let result = cli::search::run(
        &project_dir,
        &SearchQuery {
            query: Some("Foo"),
            symbol_types: &[],
            fqn_mode: false,
            regex_mode: false,
            limit: 10,
            dependency: None,
            access_levels: &[],
            offset: 0,
            scope: None,
        },
    );
    let err = result.unwrap_err();
    let cli_err = err.downcast_ref::<CliError>().unwrap();

    assert_eq!(cli_err.error_code, "INDEX_NOT_FOUND");
    assert!(!cli_err.retryable);
    assert!(
        cli_err.suggested_command.is_some(),
        "INDEX_NOT_FOUND should have a suggested_command"
    );
    assert_eq!(
        cli_err.suggested_command.as_deref(),
        Some("classpath-surfer refresh"),
        "suggested_command should be 'classpath-surfer refresh'"
    );
}

#[test]
fn agentic_exit_codes() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    std::fs::create_dir_all(&project_dir).unwrap();

    let result = cli::search::run(
        &project_dir,
        &SearchQuery {
            query: Some("Foo"),
            symbol_types: &[],
            fqn_mode: false,
            regex_mode: false,
            limit: 10,
            dependency: None,
            access_levels: &[],
            offset: 0,
            scope: None,
        },
    );
    let err = result.unwrap_err();
    let cli_err = err.downcast_ref::<CliError>().unwrap();
    assert_eq!(
        cli_err.exit_code, 3,
        "INDEX_NOT_FOUND should have exit code 3"
    );
    assert_eq!(cli_err.error_code, "INDEX_NOT_FOUND");
}

// ---------------------------------------------------------------------------
// Scope feature tests
// ---------------------------------------------------------------------------

#[test]
fn manifest_scopes_by_gav() {
    use classpath_surfer::manifest::{
        ClasspathManifest, ConfigurationManifest, DependencyInfo, ModuleManifest,
    };
    use std::collections::BTreeSet;

    let make_dep = |group: &str, artifact: &str, version: &str, scope: &str| DependencyInfo {
        group: group.to_string(),
        artifact: artifact.to_string(),
        version: version.to_string(),
        jar_path: PathBuf::from("/fake.jar"),
        source_jar_path: None,
        scope: scope.to_string(),
    };

    let manifest = ClasspathManifest {
        gradle_version: "8.14".to_string(),
        extraction_timestamp: "2024-01-01".to_string(),
        modules: vec![ModuleManifest {
            module_path: ":app".to_string(),
            configurations: vec![
                ConfigurationManifest {
                    name: "compileClasspath".to_string(),
                    dependencies: vec![
                        make_dep(
                            "com.google.guava",
                            "guava",
                            "33.4.0-jre",
                            "compileClasspath",
                        ),
                        make_dep("com.google.code.gson", "gson", "2.11.0", "compileClasspath"),
                    ],
                },
                ConfigurationManifest {
                    name: "runtimeClasspath".to_string(),
                    dependencies: vec![
                        make_dep(
                            "com.google.guava",
                            "guava",
                            "33.4.0-jre",
                            "runtimeClasspath",
                        ),
                        make_dep("org.slf4j", "slf4j-api", "2.0.0", "runtimeClasspath"),
                    ],
                },
            ],
        }],
    };

    let scope_map = manifest.scopes_by_gav();

    // guava appears in both
    let guava_scopes = scope_map.get("com.google.guava:guava:33.4.0-jre").unwrap();
    assert_eq!(
        *guava_scopes,
        BTreeSet::from([
            "compileClasspath".to_string(),
            "runtimeClasspath".to_string()
        ])
    );

    // gson is compile-only
    let gson_scopes = scope_map.get("com.google.code.gson:gson:2.11.0").unwrap();
    assert_eq!(
        *gson_scopes,
        BTreeSet::from(["compileClasspath".to_string()])
    );

    // slf4j is runtime-only
    let slf4j_scopes = scope_map.get("org.slf4j:slf4j-api:2.0.0").unwrap();
    assert_eq!(
        *slf4j_scopes,
        BTreeSet::from(["runtimeClasspath".to_string()])
    );
}

#[test]
fn search_result_scopes_serialization() {
    use classpath_surfer::model::{SearchResult, SignatureDisplay, SymbolKind};

    let result = SearchResult {
        gav: "com.google.guava:guava:33.4.0-jre".to_string(),
        symbol_kind: SymbolKind::Class,
        fqn: "com.google.common.collect.ImmutableList".to_string(),
        simple_name: "ImmutableList".to_string(),
        signature: SignatureDisplay {
            java: "public abstract class ImmutableList<E>".to_string(),
            kotlin: None,
        },
        access_flags: "public abstract".to_string(),
        source: "source_jar".to_string(),
        source_language: None,
        scopes: vec![
            "compileClasspath".to_string(),
            "runtimeClasspath".to_string(),
        ],
    };

    let json = serde_json::to_value(&result).unwrap();
    let scopes = json["scopes"].as_array().unwrap();
    assert_eq!(scopes.len(), 2);
    assert_eq!(scopes[0], "compileClasspath");
    assert_eq!(scopes[1], "runtimeClasspath");
}
