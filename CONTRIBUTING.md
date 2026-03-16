# Contributing to classpath-surfer

Thank you for your interest in contributing to classpath-surfer! This guide will help you get started.

## Development Setup

- **Rust 1.94+** (MSRV) — install via [rustup](https://rustup.rs/)
- **protoc** (Protocol Buffers compiler) — required for building (`brew install protobuf` / `apt install protobuf-compiler`)
- **JDK** (optional) — required for end-to-end tests that invoke Gradle
- **[mise](https://mise.jdx.dev/)** (optional) — for automatic toolchain management

## Build & Test

```bash
cargo build              # Build the project (includes proto compilation)
cargo clippy             # Lint
cargo fmt -- --check     # Check formatting
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps  # Doc build verification
```

### Running Tests

```bash
# Unit + pure logic (no JDK required, runs everywhere)
cargo test --test unit

# Integration (JDK 21 required)
cargo test --test integration --test integration_mutation --test cli

# JDK/Gradle compatibility matrix (CI runs this automatically)
JAVA_17_HOME=/path/to/jdk17 cargo test --test matrix -- --test-threads=1
```

All checks must pass before submitting a PR.

## Architecture Overview

classpath-surfer follows a pipeline architecture:

1. **Gradle init script** (`src/gradle/`) — injected into the target project's Gradle build to extract the full classpath (all resolved dependencies with GAV coordinates and JAR paths).
2. **Classpath extraction** (`src/manifest/`) — the extracted classpath is modeled as a manifest; incremental updates are computed by diffing against the previous manifest at the GAV level.
3. **JAR parsing** (`src/parser/`) — each JAR is scanned; `.class` files are parsed using the `cafebabe` crate to extract class names, method signatures, and field declarations. For Kotlin classes, the `@kotlin.Metadata` annotation is decoded via protobuf (prost) to produce Kotlin-native signatures. The `SourceFile` attribute is used to detect the source language.
4. **Tantivy indexing** (`src/index/`) — extracted symbols are written to a Tantivy full-text search index supporting text, fully-qualified name (FQN), and regex queries.
5. **Source resolution** (`src/source/`) — source JARs are resolved when available; otherwise, classes are decompiled on the fly using CFR or Vineflower.
6. **Output rendering** (`src/cli/`, `src/tui/`, `src/output.rs`) — three output modes (Agentic JSON, interactive TUI with syntect-based syntax highlighting, Plain text) are selected based on the `--agentic` flag and TTY detection.
7. **Staleness detection** (`src/staleness/`) — lockfile hashes and build-file mtimes are compared against the snapshot taken at index time to detect when a `refresh` is needed.
8. **Error handling** (`src/error.rs`) — `CliError` provides classified exit codes (0/1/2/3), machine-readable error codes, and a retryable flag for agent integration.

## Adding a New Subcommand

1. Create a new module in `src/cli/` (e.g., `src/cli/my_command.rs`).
2. Add a variant to the `Commands` enum in `src/cli/mod.rs`.
3. Define an output struct `XxxOutput` in `src/model/output.rs` with `#[derive(Serialize)]`, and have the handler return `Result<XxxOutput>`.
4. Add a Plain text renderer in `src/cli/render.rs`.
5. Add a TUI renderer in `src/tui/` (if the command benefits from interactive display).
6. Add OutputMode dispatch in `main.rs` (Agentic → JSON, TUI → ratatui, Plain → render).

## Index Schema Changes

If you modify the Tantivy index schema in `src/index/schema.rs`, you **must** update the reader (`src/index/reader.rs`), writer (`src/index/writer.rs`), and the `REQUIRED_FIELDS` constant in `src/index/compat.rs` to stay in sync.

## Code Style

- **Rust edition 2024**, MSRV 1.94
- Use `anyhow::Result` as the return type for all fallible functions.
- User-facing messages go to **stderr** (`eprintln!`); data output goes to **stdout** (`println!`).
- Use **clap derive macros** for CLI argument parsing.
- `#![deny(missing_docs)]` is enabled — all `pub` items must have `///` doc comments.
- User-facing documentation (README, CONTRIBUTING, GitHub templates) is maintained in both English and Korean.

## PR Process

1. Fork the repository and create a feature branch.
2. Make your changes.
3. Ensure all checks pass: `cargo fmt -- --check && cargo clippy -- -D warnings && cargo test --test unit`.
4. Open a pull request against `master`.

## Project Structure

```
project root
├── .claude-plugin/      # Claude Code plugin manifest & marketplace config
├── agents/              # Claude Code agent definitions (find-symbol, show-source)
├── skills/              # Claude Code skill definitions (SKILL.md)
├── build.rs             # Proto compilation (prost-build)
├── proto/               # Kotlin metadata protobuf schema
├── scripts/             # Helper scripts (proto sync)
├── vendor/              # Vendored Kotlin syntax definition (syntect)
└── src/
    ├── main.rs          # CLI entrypoint (clap)
    ├── cli/             # Subcommand handlers + plain-text renderers
    ├── config.rs        # .classpath-surfer/config.json
    ├── error.rs         # Classified CLI error types (exit codes, error codes)
    ├── gradle/          # Init script & Gradle runner
    ├── index/           # Tantivy schema, reader, writer
    ├── manifest/        # Classpath manifest model, merge, diff
    ├── model/           # Core types (SymbolDoc, SearchResult, SourceProvider, *Output)
    ├── output.rs        # Output mode detection (Agentic/TUI/Plain)
    ├── parser/          # JAR / .class / descriptor / Kotlin metadata parsing
    ├── source/          # Source resolver & decompiler integration
    ├── staleness/       # Lockfile & build-file change detection
    └── tui/             # Interactive TUI renderers (ratatui + syntect)
```

## Project Root Files

| Path | Description |
|------|-------------|
| `build.rs` | Proto compilation via prost-build (runs automatically during `cargo build`) |
| `proto/` | Kotlin metadata protobuf schema (`kotlin_metadata.proto`) |
| `scripts/` | Helper scripts (e.g., `sync-kotlin-proto.sh` for upstream proto sync) |
| `vendor/` | Vendored assets (Kotlin syntax definition for syntect) |

## Exit Code Conventions

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General failure |
| 2 | Usage error (invalid arguments) |
| 3 | Resource not found (e.g., index does not exist) |
