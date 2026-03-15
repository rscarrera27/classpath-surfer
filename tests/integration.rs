mod common;

use classpath_surfer::cli;
use classpath_surfer::index::reader::IndexReader;
use classpath_surfer::model::SearchQuery;
use classpath_surfer::source::resolver;
use common::require_indexed_project;

// ---------------------------------------------------------------------------
// Shared-index integration tests (JDK 21, read-only)
// ---------------------------------------------------------------------------

#[test]
fn kotlin_metadata_extraction() {
    let project = require_indexed_project!();
    let reader = IndexReader::open(&project.index_dir()).expect("index should be readable");

    // CoroutineScope: should be an interface
    let (results, _count) = reader
        .search(&SearchQuery {
            query: "CoroutineScope",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 10,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .expect("search should succeed");
    assert!(
        !results.is_empty(),
        "CoroutineScope should be found from kotlinx-coroutines"
    );
    let scope = &results[0];
    assert_eq!(
        scope.source_language.map(|l| l.to_string()).as_deref(),
        Some("kotlin")
    );
    assert!(scope.signature.kotlin.is_some());
    let sig = scope.signature.kotlin.as_deref().unwrap();
    assert!(
        sig.contains("interface"),
        "CoroutineScope should be an interface, got: {sig}"
    );

    // Deferred: interface with type parameter
    let (results, _count) = reader
        .search(&SearchQuery {
            query: "Deferred",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 10,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .expect("search should succeed");
    let deferred = results
        .iter()
        .find(|r| r.source_language.map(|l| l.to_string()).as_deref() == Some("kotlin"));
    assert!(
        deferred.is_some(),
        "Deferred should be found as Kotlin class"
    );

    // Job: interface
    let (results, _count) = reader
        .search(&SearchQuery {
            query: "Job",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 10,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .expect("search should succeed");
    let job = results.iter().find(|r| {
        r.source_language.map(|l| l.to_string()).as_deref() == Some("kotlin")
            && r.fqn.contains("kotlinx.coroutines")
    });
    assert!(
        job.is_some(),
        "Job interface should be found from kotlinx-coroutines"
    );
}

#[test]
fn kotlin_jvm_symbols() {
    let project = require_indexed_project!();
    let reader = IndexReader::open(&project.index_dir()).unwrap();

    // kotlinx-serialization: Json class
    let (results, _count) = reader
        .search(&SearchQuery {
            query: "Json",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    let json_class = results
        .iter()
        .find(|r| r.fqn.contains("kotlinx.serialization.json"));
    assert!(
        json_class.is_some(),
        "kotlinx.serialization.json.Json should be found"
    );
    assert_eq!(
        json_class
            .unwrap()
            .source_language
            .map(|l| l.to_string())
            .as_deref(),
        Some("kotlin")
    );

    // kotlinx-serialization: Serializable annotation (transitive from core)
    let (results, _count) = reader
        .search(&SearchQuery {
            query: "Serializable",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    let serializable = results
        .iter()
        .find(|r| r.fqn.contains("kotlinx.serialization"));
    assert!(
        serializable.is_some(),
        "kotlinx.serialization.Serializable should be found (transitive)"
    );

    // Ktor: HttpClient class
    let (results, _count) = reader
        .search(&SearchQuery {
            query: "HttpClient",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    let ktor = results.iter().find(|r| r.fqn.contains("io.ktor.client"));
    assert!(ktor.is_some(), "io.ktor.client.HttpClient should be found");
}

#[test]
fn kmp_jvm_symbols() {
    let project = require_indexed_project!();
    let reader = IndexReader::open(&project.index_dir()).unwrap();

    // kotlinx-datetime
    let (results, _count) = reader
        .search(&SearchQuery {
            query: "Instant",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    let instant = results.iter().find(|r| r.fqn.contains("kotlinx.datetime"));
    assert!(
        instant.is_some(),
        "kotlinx.datetime.Instant should be found"
    );

    let (results, _count) = reader
        .search(&SearchQuery {
            query: "Clock",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    let clock = results.iter().find(|r| r.fqn.contains("kotlinx.datetime"));
    assert!(clock.is_some(), "kotlinx.datetime.Clock should be found");

    // kotlinx-io
    let (results, _count) = reader
        .search(&SearchQuery {
            query: "Buffer",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    let buffer = results.iter().find(|r| r.fqn.contains("kotlinx.io"));
    assert!(buffer.is_some(), "kotlinx.io.Buffer should be found");

    let (results, _count) = reader
        .search(&SearchQuery {
            query: "Source",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    let source = results.iter().find(|r| r.fqn.contains("kotlinx.io"));
    assert!(source.is_some(), "kotlinx.io.Source should be found");
}

#[test]
fn annotation_processor_symbols() {
    let project = require_indexed_project!();
    let reader = IndexReader::open(&project.index_dir()).unwrap();

    let (results, _count) = reader
        .search(&SearchQuery {
            query: "Component",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    let dagger = results.iter().find(|r| r.fqn == "dagger.Component");
    assert!(dagger.is_some(), "dagger.Component should be found");

    let (results, _count) = reader
        .search(&SearchQuery {
            query: "Module",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    let module = results.iter().find(|r| r.fqn == "dagger.Module");
    assert!(module.is_some(), "dagger.Module should be found");

    let (results, _count) = reader
        .search(&SearchQuery {
            query: "Provides",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    let provides = results.iter().find(|r| r.fqn == "dagger.Provides");
    assert!(provides.is_some(), "dagger.Provides should be found");
}

#[test]
fn large_library_symbols() {
    let project = require_indexed_project!();
    let reader = IndexReader::open(&project.index_dir()).unwrap();

    // Spring Core
    let (results, _count) = reader
        .search(&SearchQuery {
            query: "Environment",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    let spring = results.iter().find(|r| r.fqn.contains("springframework"));
    assert!(
        spring.is_some(),
        "org.springframework.core.env.Environment should be found"
    );

    // OkHttp
    let (results, _count) = reader
        .search(&SearchQuery {
            query: "OkHttpClient",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    let okhttp = results.iter().find(|r| r.fqn.contains("okhttp3"));
    assert!(okhttp.is_some(), "okhttp3.OkHttpClient should be found");

    let count = reader.count_symbols().unwrap();
    assert!(
        count > 1000,
        "Expected at least 1000 symbols with expanded deps, got {count}"
    );
}

#[test]
fn interface_only_symbols() {
    let project = require_indexed_project!();
    let reader = IndexReader::open(&project.index_dir()).unwrap();

    // SLF4J
    let (results, _count) = reader
        .search(&SearchQuery {
            query: "Logger",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    let slf4j = results.iter().find(|r| r.fqn == "org.slf4j.Logger");
    assert!(slf4j.is_some(), "org.slf4j.Logger should be found");

    let (results, _count) = reader
        .search(&SearchQuery {
            query: "LoggerFactory",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    let factory = results.iter().find(|r| r.fqn == "org.slf4j.LoggerFactory");
    assert!(factory.is_some(), "org.slf4j.LoggerFactory should be found");

    // Jakarta Servlet
    let (results, _count) = reader
        .search(&SearchQuery {
            query: "HttpServlet",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    let servlet = results.iter().find(|r| r.fqn.contains("jakarta.servlet"));
    assert!(
        servlet.is_some(),
        "jakarta.servlet.http.HttpServlet should be found"
    );

    let (results, _count) = reader
        .search(&SearchQuery {
            query: "Filter",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    let filter = results.iter().find(|r| r.fqn == "jakarta.servlet.Filter");
    assert!(filter.is_some(), "jakarta.servlet.Filter should be found");
}

#[test]
fn search_no_results() {
    let project = require_indexed_project!();

    let output = cli::search::run(
        &project.project_dir,
        &SearchQuery {
            query: "XyzNonExistentClassName12345",
            symbol_type: "any",
            fqn_mode: false,
            regex_mode: false,
            limit: 10,
            dependency: None,
            access_levels: None,
            offset: 0,
        },
    )
    .expect("search with no results should succeed (not error)");
    assert!(
        output.results.is_empty(),
        "should return empty results, not an error"
    );
}

#[test]
fn agentic_search_output_fields() {
    let project = require_indexed_project!();

    let output = cli::search::run(
        &project.project_dir,
        &SearchQuery {
            query: "ImmutableList",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 10,
            dependency: None,
            access_levels: None,
            offset: 0,
        },
    )
    .expect("search should succeed");

    let json = serde_json::to_value(&output).unwrap();
    assert!(
        json.get("query").is_some(),
        "JSON should have 'query' field"
    );
    assert!(
        json.get("total_matches").is_some(),
        "JSON should have 'total_matches' field"
    );
    assert!(
        json.get("results").is_some(),
        "JSON should have 'results' field"
    );

    let total_matches = json["total_matches"].as_u64().unwrap();
    let results = json["results"].as_array().unwrap();
    assert!(!results.is_empty());
    assert!(
        total_matches >= results.len() as u64,
        "total_matches ({total_matches}) should be >= results count ({})",
        results.len()
    );

    let first = &results[0];
    assert!(first.get("fqn").is_some());
    assert!(first.get("symbol_kind").is_some());
    assert!(first.get("gav").is_some());
    assert!(first.get("simple_name").is_some());
    assert!(first.get("signature_java").is_some());
    assert!(first.get("access_flags").is_some());
    assert!(first.get("source").is_some());
    assert!(
        first.get("score").is_none(),
        "score field should not be present"
    );
}

#[test]
fn status_after_refresh() {
    let project = require_indexed_project!();

    let status = cli::status::run(&project.project_dir).expect("status should succeed");

    assert!(status.initialized);
    assert!(status.has_index);
    assert!(status.dependency_count > 0);
    assert!(status.indexed_symbols.is_some() && status.indexed_symbols.unwrap() > 0);
    assert!(!status.is_stale);
    assert!(status.index_size.is_some());
    assert!(status.with_source_jars > 0);
}

#[test]
fn show_no_source_fails_with_no_decompile() {
    let project = require_indexed_project!();

    let manifest = common::read_manifest(&project.project_dir);

    // Find a dependency that has no source JAR
    let deps = manifest.all_dependencies();
    let no_source_dep = deps.iter().find(|d| d.source_jar_path.is_none());

    if let Some(dep) = no_source_dep {
        // Try to find a class FQN from this dependency
        let reader = IndexReader::open(&project.index_dir()).unwrap();
        let (results, _count) = reader
            .search(&SearchQuery {
                query: "*",
                symbol_type: "class",
                fqn_mode: false,
                regex_mode: false,
                limit: 5,
                dependency: Some(&dep.gav()),
                access_levels: None,
                offset: 0,
            })
            .ok()
            .unwrap_or_default();

        if let Some(result) = results.first() {
            let result = resolver::resolve_source(
                &result.fqn,
                &project.project_dir,
                &manifest,
                "cfr",
                None,
                true, // no_decompile = true
            );
            assert!(
                result.is_err(),
                "resolve_source with no_decompile=true should fail when no source JAR is available"
            );
        } else {
            eprintln!("No class found for dependency without source, skipping assertion");
        }
    } else {
        // All deps have sources -- verify resolve_source works for one
        let result = resolver::resolve_source(
            "com.google.common.collect.ImmutableList",
            &project.project_dir,
            &manifest,
            "cfr",
            None,
            true, // no_decompile, but source jar should exist
        );
        assert!(
            result.is_ok(),
            "resolve_source should succeed for class with source JAR"
        );
        eprintln!("All deps have source JARs; verified source resolution instead");
    }
}

#[test]
fn show_with_source_jar() {
    let project = require_indexed_project!();

    let output = cli::show::run(
        &project.project_dir,
        "com.google.gson.Gson",
        "cfr",
        None,
        true, // no_decompile
    )
    .expect("show should succeed for Gson (has source JAR)");

    assert_eq!(output.fqn, "com.google.gson.Gson");
    assert!(!output.gav.is_empty());
    assert!(
        output.primary.content.contains("class Gson"),
        "source should contain 'class Gson'"
    );
    assert_eq!(output.primary.language, "java");
    assert!(output.primary.source.has_source());
}

#[test]
fn scala_clojure_symbols() {
    let project = require_indexed_project!();
    let reader = IndexReader::open(&project.index_dir()).expect("index should be readable");

    // Scala: scala.Option should be indexed with source_language == "scala"
    let (results, _count) = reader
        .search(&SearchQuery {
            query: "Option",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .expect("search should succeed");
    let scala_option = results
        .iter()
        .find(|r| r.fqn == "scala.Option" || r.fqn.starts_with("scala.Option"));
    assert!(
        scala_option.is_some(),
        "scala.Option should be found from scala-library"
    );
    assert_eq!(
        scala_option
            .unwrap()
            .source_language
            .map(|l| l.to_string())
            .as_deref(),
        Some("scala"),
        "scala.Option source_language should be 'scala'"
    );

    // Clojure: clojure.lang.PersistentVector is written in Java
    let (results, _count) = reader
        .search(&SearchQuery {
            query: "PersistentVector",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .expect("search should succeed");
    let clj_pv = results
        .iter()
        .find(|r| r.fqn.contains("clojure.lang.PersistentVector"));
    assert!(
        clj_pv.is_some(),
        "clojure.lang.PersistentVector should be found from clojure"
    );
    assert_eq!(
        clj_pv
            .unwrap()
            .source_language
            .map(|l| l.to_string())
            .as_deref(),
        Some("java"),
        "clojure.lang.PersistentVector source_language should be 'java' (written in Java)"
    );
}

// ---------------------------------------------------------------------------
// deps: list dependencies
// ---------------------------------------------------------------------------

#[test]
fn deps_lists_all_dependencies() {
    let project = require_indexed_project!();
    let output = cli::deps::run(&project.project_dir, None, 200, 0).expect("deps should succeed");
    assert!(
        output.total_count > 0,
        "should have at least one dependency"
    );
    assert!(!output.dependencies.is_empty());
    for dep in &output.dependencies {
        assert!(
            dep.symbol_count > 0,
            "each dep should have symbols: {}",
            dep.gav
        );
    }
}

#[test]
fn deps_filter() {
    let project = require_indexed_project!();
    let output = cli::deps::run(&project.project_dir, Some("com.google.*:*"), 200, 0)
        .expect("deps with filter should succeed");
    assert!(
        output.total_count > 0,
        "should have at least one com.google dependency"
    );
    for dep in &output.dependencies {
        assert!(
            dep.gav.starts_with("com.google."),
            "filtered dep should match: {}",
            dep.gav
        );
    }
}

#[test]
fn deps_pagination() {
    let project = require_indexed_project!();
    let output = cli::deps::run(&project.project_dir, None, 2, 0)
        .expect("deps with small limit should succeed");
    assert!(output.dependencies.len() <= 2);
    if output.total_count > 2 {
        assert!(output.has_more, "should have more results");
    }
}

// ---------------------------------------------------------------------------
// list: list symbols for a dependency
// ---------------------------------------------------------------------------

#[test]
fn list_symbols_for_dependency() {
    let project = require_indexed_project!();
    let output = cli::list::run(
        &project.project_dir,
        "com.google.code.gson:gson:*",
        &["class", "method"],
        Some(&["public"]),
        50,
        0,
    )
    .expect("list should succeed");
    assert!(
        !output.matched_gavs.is_empty(),
        "should match at least one GAV"
    );
    assert!(
        !output.symbols.is_empty(),
        "gson should have public symbols"
    );
}

#[test]
fn list_default_type_filter() {
    let project = require_indexed_project!();
    let output = cli::list::run(
        &project.project_dir,
        "com.google.code.gson:gson:*",
        &["class", "method"],
        Some(&["public"]),
        200,
        0,
    )
    .expect("list should succeed");
    for sym in &output.symbols {
        assert!(
            sym.symbol_kind == classpath_surfer::model::SymbolKind::Class
                || sym.symbol_kind == classpath_surfer::model::SymbolKind::Method,
            "only class/method should be returned, got: {:?}",
            sym.symbol_kind
        );
    }
}

#[test]
fn list_pagination() {
    let project = require_indexed_project!();
    let page1 = cli::list::run(
        &project.project_dir,
        "com.google.code.gson:gson:*",
        &["class", "method"],
        Some(&["public"]),
        5,
        0,
    )
    .expect("list page 1 should succeed");

    if page1.total_symbols > 5 {
        assert!(page1.has_more, "should have more results");

        let page2 = cli::list::run(
            &project.project_dir,
            "com.google.code.gson:gson:*",
            &["class", "method"],
            Some(&["public"]),
            5,
            5,
        )
        .expect("list page 2 should succeed");
        assert!(!page2.symbols.is_empty(), "page 2 should have results");
    }
}

// ---------------------------------------------------------------------------
// Smart search: token AND + auto FQN detection
// ---------------------------------------------------------------------------

#[test]
fn smart_search_multi_keyword_and() {
    let project = require_indexed_project!();
    let reader = IndexReader::open(&project.index_dir()).unwrap();

    // "immutable list" — both must be substrings
    let (results, _count) = reader
        .search(&SearchQuery {
            query: "immutable list",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    assert!(
        results.iter().any(|r| r.fqn.contains("ImmutableList")),
        "multi-keyword 'immutable list' should find ImmutableList, got: {:?}",
        results.iter().map(|r| &r.fqn).collect::<Vec<_>>()
    );
}

#[test]
fn smart_search_auto_fqn() {
    let project = require_indexed_project!();
    let reader = IndexReader::open(&project.index_dir()).unwrap();

    // FQN with 2+ dots should auto-detect as exact FQN match
    let (results, _count) = reader
        .search(&SearchQuery {
            query: "com.google.common.collect.ImmutableList",
            symbol_type: "class",
            fqn_mode: false,
            regex_mode: false,
            limit: 10,
            dependency: None,
            access_levels: None,
            offset: 0,
        })
        .unwrap();
    assert!(
        !results.is_empty(),
        "auto-FQN should find com.google.common.collect.ImmutableList"
    );
    assert_eq!(
        results[0].fqn, "com.google.common.collect.ImmutableList",
        "first result should be exact FQN match"
    );
}
