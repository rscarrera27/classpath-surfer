<div align="center">

# classpath-surfer

**Fast dependency symbol search for Gradle Java/Kotlin projects**

[![CI](https://github.com/rscarrera27/classpath-surfer/actions/workflows/ci.yml/badge.svg)](https://github.com/rscarrera27/classpath-surfer/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.94%2B-orange.svg)](https://www.rust-lang.org/)

Index every class, method, and field from your resolved classpath,<br>
then search them instantly — from the CLI or directly inside [Claude Code](https://claude.ai/claude-code).

[English](README.md) | [한국어](README_ko.md)

<!-- TODO: Add demo GIF here — record TUI search + show with asciinema + agg -->

</div>

> [!WARNING]
> This project is in **alpha** stage. APIs, CLI flags, index format, and configuration schema are subject to breaking changes without notice. Use at your own risk and expect rough edges. Bug reports and feedback are very welcome!

---

## Why

Coding agents (like Claude Code) working on Gradle Java/Kotlin projects repeatedly struggle with external library symbols — blindly crawling `~/.gradle/caches/`, accessing artifacts outside the actual classpath, decompiling classes when source JARs exist, and rediscovering this fragile workflow from scratch every time.

**classpath-surfer** solves this by building a local [Tantivy](https://github.com/quickwit-oss/tantivy) full-text index over all symbols in your resolved classpath. Agents (and humans) can search symbols and read source code instantly.

## Features

| | Feature | Description |
|---|---------|-------------|
| :mag: | **Symbol search** | Smart search with auto FQN detection, CamelCase token splitting, and prefix matching — filter by kind, dependency, access level, or classpath |
| :zap: | **Fast indexing** | Auto-extract Gradle project classpath and index all symbols in seconds; incremental updates via GAV-level diff |
| :globe_with_meridians: | **Kotlin signatures** | Decode `@kotlin.Metadata` protobuf to display native Kotlin signatures like `suspend fun` and `data class` |
| :package: | **JVM-language agnostic** | Java, Kotlin, Scala, Groovy, Clojure — search symbols from dependencies written in any JVM language |
| :page_facing_up: | **Source code lookup** | Auto-focus on a symbol with surrounding context; source JARs when available, otherwise on-demand CFR/Vineflower decompilation — Kotlin shows original `.kt` source with a secondary decompiled Java view |
| :robot: | **AI agent integration** | `--agentic` JSON output with classified exit codes for any AI agent; optional Claude Code plugin for slash-command skills |

## Quick Start

### Install

> [!NOTE]
> Pre-built binaries are currently available for **Apple Silicon (aarch64) only**. For other platforms or building from source, see [CONTRIBUTING.md](CONTRIBUTING.md).

```bash
brew install rscarrera27/tap/classpath-surfer
```

### Set up a project

```bash
cd your-gradle-project
classpath-surfer index init      # writes config, Gradle init script, then runs initial refresh

# Verify the index was built
classpath-surfer index status
# → 38 dependencies, 77,219 symbols indexed, 2.1 MB on disk
```

### Find a symbol

```bash
# Search by name
classpath-surfer search symbol ImmutableList

# CamelCase token matching — finds ImmutableList, ImmutableMap, ImmutableSet, etc.
classpath-surfer search symbol Immutable

# Find coroutine launchers in kotlinx-coroutines
classpath-surfer search symbol launch --type method --dependency "org.jetbrains.kotlinx:*"

# List all symbols in a specific dependency
classpath-surfer search symbol --dependency "com.google.guava:guava"

# Glob search for HTTP client classes
classpath-surfer search symbol "Http*Client" --type class

# Filter by classpath (compile, runtime, testCompile, testRuntime, ...)
classpath-surfer search symbol Annotation --type class --classpath compile

# FQN-like queries are auto-detected
classpath-surfer search symbol com.google.common.collect.ImmutableList
```

### Read the source

```bash
# Show source — auto-focuses on the symbol with 25 lines of context
classpath-surfer show com.google.common.collect.ImmutableList
classpath-surfer show com.google.common.collect.ImmutableList.of

# Widen context / show the full file
classpath-surfer show com.google.common.collect.ImmutableList --context 50
classpath-surfer show com.google.common.collect.ImmutableList --full

# Kotlin sources display original .kt files (suspend fun, data class, etc.)
classpath-surfer show kotlinx.coroutines.CoroutineScope
```

If a `-sources.jar` is available it will be used; otherwise the class is decompiled with CFR (default) or Vineflower.

### Browse dependencies

```bash
# List indexed dependencies with symbol counts and classpaths
classpath-surfer search dep

# Filter by GAV pattern
classpath-surfer search dep "io.netty:*"

# Show only runtime dependencies
classpath-surfer search dep --classpath runtime
```

### Browse packages

```bash
# List indexed Java packages with symbol counts
classpath-surfer search pkg

# Filter by package pattern
classpath-surfer search pkg "com.google.*"

# Packages from a specific dependency
classpath-surfer search pkg --dependency "com.google.code.gson:gson:*"

# Filter by classpath
classpath-surfer search pkg --classpath compile
```

### AI agent / script integration

```bash
# All commands support --agentic for structured JSON output
classpath-surfer search symbol ImmutableList --agentic
classpath-surfer show com.google.common.collect.ImmutableList --agentic

# Non-TTY automatically outputs plain text (pipe-friendly)
classpath-surfer search symbol ImmutableList | head
```

## Commands

| Command | Description |
|---------|-------------|
| `search symbol <query>` | Search for symbols in the index |
| `search dep [pattern]` | List indexed dependencies with symbol counts |
| `search pkg [pattern]` | List indexed Java packages |
| `show <fqn>` | Display source code for a symbol (focuses on the target symbol by default) |
| `index init` | Install Gradle init script, default config, and run initial refresh |
| `index refresh` | Extract classpath via Gradle and build/update the symbol index (skips Gradle when fresh; use `--force` to override) |
| `index status` | Show index stats (dependency count, symbol count, staleness, disk size) |
| `index clean` | Remove index data |
| `--agentic` | Global flag: emit structured JSON output for AI agents and scripts |

## Performance

Benchmarked on Macbook Pro 2023(M2 Pro, 32GB) with a 38-dependency project (77,219 symbols including Guava, Spring Core, Ktor, kotlinx-coroutines, OkHttp, and more):

### Search latency

| Query type | Latency |
|-----------|---------|
| Simple keyword (`ImmutableList`) | **83 µs** |
| FQN exact match | **10 µs** |
| Regex (`Immutable.*`) | **350 µs** |
| With type filter | **73 µs** |
| With dependency filter | **92 µs** |

### Indexing speed

| Operation | Time |
|-----------|------|
| Full refresh (38 deps, 77K symbols) | **1.76 s** |
| Incremental refresh (1 dep removed) | **607 ms** |
| No-op refresh (up to date) | **537 ms** |

<details>
<summary>Reproduce these benchmarks</summary>

```bash
cargo bench --bench search
cargo bench --bench refresh
```

</details>

## How It Works

```mermaid
graph TD
    A["Gradle project\n(gradlew)"] -- "init script injects\nclasspathSurferExport task" --> B["Classpath\nmanifest.json"]
    B --> C1["JAR #1"]
    B --> C2["JAR #2"]
    B --> CN["JAR #N"]
    C1 -- "cafebabe .class\n+ Kotlin metadata" --> D
    C2 -- "parse symbols" --> D
    CN --> D
    D["Tantivy full-text index\nFQN · name · camelCase tokens"]
    D --> E1["search\n(TUI / plain / JSON)"]
    D --> E2["show\n(source)"]
    D --> E3["status\n(staleness)"]
```

1. **Extract** — A Gradle init script resolves `compileClasspath` and `runtimeClasspath` for every subproject, writing a per-module JSON manifest with each dependency's GAV coordinates and JAR paths (including source JARs when available).
2. **Parse** — Each JAR is opened with the `cafebabe` crate. Every `.class` file is parsed to extract class names, methods, fields, descriptors, and access flags. For Kotlin classes, the `@kotlin.Metadata` annotation is decoded via protobuf (prost) to produce Kotlin-native signatures. The `SourceFile` attribute is used to detect the source language.
3. **Index** — Extracted symbols are written into a Tantivy index with fields for FQN, simple name, camelCase-split tokens, kind, signature, and GAV.
4. **Search** — Queries hit the Tantivy index. Results are ranked by relevance and returned as a table or JSON.
5. **Staleness** — On each search, the tool checks lockfile hashes and build-file mtimes against the snapshot taken at index time. If anything changed, it asks you to `index refresh`.

## Claude Code Integration

classpath-surfer ships as a [Claude Code plugin](https://claude.ai/claude-code) with three skills that mirror CLI commands:

| Skill | Usage |
|-------|-------|
| `/search-classpath <query>` | Search for symbols, dependencies, or packages |
| `/show-classpath-source <fqn>` | Show source code for a fully qualified symbol |
| `/classpath-index [action]` | Manage the symbol index (init, refresh, status, clean, diagnose) |

### Install the plugin

```bash
# Inside Claude Code
/plugin marketplace add https://github.com/rscarrera27/classpath-surfer
/plugin install classpath-surfer
```

This lets Claude Code discover and read dependency APIs without you having to look them up manually.

## Configuration

`classpath-surfer index init` writes `.classpath-surfer/config.json`:

```json
{
  "decompiler": "cfr",
  "decompiler_jar": null,
  "configurations": ["compileClasspath", "runtimeClasspath"],
  "java_home": null,
  "no_decompile": false
}
```

| Field | Description |
|-------|-------------|
| `decompiler` | `"cfr"` or `"vineflower"` |
| `decompiler_jar` | Explicit path to the decompiler JAR. If unset, reads `CFR_JAR` or `VINEFLOWER_JAR` env var |
| `configurations` | Gradle configurations to resolve |
| `java_home` | Override `JAVA_HOME` (used to run the decompiler) |
| `no_decompile` | `false` (default). When `true`, fail instead of decompiling if no source JAR |

CLI flags (`--decompiler`, `--configurations`, `--no-decompile`) override config file values when provided.

## Requirements

- **Gradle** project with `gradlew` (or `gradle` on `PATH`)
- **JDK** (only needed for decompilation via `show`)

For build-from-source requirements, see [CONTRIBUTING.md](CONTRIBUTING.md).

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

---

<div align="center">
<sub>Designed by a human. Built by <a href="https://claude.ai/claude-code">Claude Code</a>.</sub>
</div>
