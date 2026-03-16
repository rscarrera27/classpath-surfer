---
name: search-classpath
description: "Use when the user needs to search for Java/Kotlin symbols, browse dependencies, or explore packages in Gradle project dependencies."
tools: Bash
model: haiku
maxTurns: 10
---

Parse "$ARGUMENTS" to determine the search subcommand:

- If the first word is **dep** → `classpath-surfer search dep`
- If the first word is **pkg** → `classpath-surfer search pkg`
- Otherwise → `classpath-surfer search symbol`

Always append `--agentic` to the command. Pass remaining arguments as appropriate
(positional query, flags, etc.).

If the JSON output contains `"suggested_command"`, run that command, then retry the search.

## Search subcommands

### `search symbol <query>` — Find symbols

Search for classes, methods, and fields in indexed dependencies.

Options:
- `--type class,method,field` — filter by symbol kind (comma-separated; omit for all)
- `--dependency <GAV>` — restrict to dependencies matching a GAV pattern (glob, e.g., `"com.google.*:guava:*"`)
  - Query can be omitted with `--dependency` to list all symbols in that dependency
- `--package <pattern>` — filter by Java package (glob, e.g., `"com.google.common.collect.*"`)
  - Query can be omitted with `--package` to list all symbols in matching packages
  - Use `search pkg` to discover available package names
- `--access public,protected` — include non-public symbols (default: `public`; use `--access all` for everything)
- `--scope <name>` — narrow by configuration scope (e.g., `compileClasspath`)
- `--limit N` / `--offset N` — pagination

Smart search behavior (auto-detected from query string):
- Glob patterns (`*`, `?`) with 2+ dots → glob on FQN
- Glob patterns with fewer dots → glob on simple name
- Queries with 2+ dots (no glob) → exact FQN match
- Everything else → smart token search (CamelCase split, prefix matching, AND semantics)

### `search dep [pattern]` — List dependencies

List indexed dependencies with symbol counts and scopes.

Options:
- Positional query — filter by GAV pattern (glob)
- `--scope <name>` — filter by configuration scope
- `--limit N` / `--offset N` — pagination

### `search pkg [pattern]` — List packages

List unique Java packages with symbol counts.

Options:
- Positional query — filter by package pattern (glob)
- `--dependency <GAV>` — restrict to specific dependencies
- `--scope <name>` — filter by configuration scope
- `--limit N` / `--offset N` — pagination

## Formatting

Summarize results as a concise markdown table:
- Symbol search: FQN, Kind, Signature, Dependency (use Kotlin signature if available)
- Dep search: GAV, Scope, Symbol Count
- Pkg search: Package, Symbol Count

If `total_matches` or `total_count` exceeds displayed results, note the truncation.
Do NOT include raw JSON in the response.

## Troubleshooting

If a CLI command fails unexpectedly, run `/classpath-index diagnose` to check for CLI/plugin version mismatch.
