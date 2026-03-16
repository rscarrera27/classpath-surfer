//! # classpath-surfer
//!
//! Fast dependency symbol search for Gradle Java/Kotlin projects.
//!
//! ## Pipeline
//!
//! 1. **Gradle extraction** ([`gradle`]) — Injects an init script into Gradle to
//!    collect the resolved classpath (GAV coordinates and JAR paths) for each module.
//! 2. **JAR / classfile parsing** ([`parser`]) — Walks `.class` files inside each JAR
//!    and extracts type, method, and field symbols along with their descriptors.
//! 3. **Tantivy indexing** ([`index`]) — Stores the extracted symbols in a local
//!    Tantivy full-text index for sub-millisecond lookups.
//! 4. **Search** ([`cli`]) — Exposes subcommands (`search symbol`, `show`, `index status`, …) that
//!    query the index and present results to the user.
//!
//! ## Module overview
//!
//! | Module | Purpose |
//! |---|---|
//! | [`cli`] | Clap-based subcommand handlers |
//! | [`config`] | Per-project configuration (decompiler, target configs) |
//! | [`gradle`] | Gradle init script generation and runner |
//! | [`index`] | Tantivy schema, reader, and writer |
//! | [`manifest`] | Classpath manifest model, merge, and diff |
//! | [`model`] | Shared domain types |
//! | [`output`] | Output mode (Agentic/TUI/Plain) detection and JSON emit helpers |
//! | [`parser`] | JAR, classfile, and descriptor parsing |
//! | [`source`] | Source-code resolution (source JARs / decompilation) |
//! | [`staleness`] | Index freshness checks (lockfile hash, buildfile mtime) |
//! | [`tui`] | Ratatui-based interactive TUI renderers |

#![deny(missing_docs)]

/// CLI subcommand dispatch.
pub mod cli;
/// Per-project configuration (decompiler, target configurations).
pub mod config;
/// Classified CLI error type with exit code, error code, and retryability.
pub mod error;
/// Gradle init script generation and execution.
pub mod gradle;
/// Tantivy-based symbol index (schema, reader, writer).
pub mod index;
/// Classpath manifest model, merge, and diff.
pub mod manifest;
/// Shared domain types (`SymbolDoc`, `SearchResult`, `SourceProvider`).
pub mod model;
/// Output mode detection and JSON emission helpers.
pub mod output;
/// JAR, classfile, and descriptor parsing.
pub mod parser;
/// Source-code resolution (source JARs / decompilation).
pub mod source;
/// Index freshness checks (lockfile hash, buildfile mtime).
pub mod staleness;
/// Ratatui-based interactive TUI renderers.
pub mod tui;
