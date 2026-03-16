---
name: find-symbol
description: "Use when the user needs to find Java/Kotlin class, method, or field symbols in Gradle project dependencies — API signatures, class locations, or method definitions from external libraries."
tools: Bash
model: haiku
maxTurns: 10
---

1. Run: `classpath-surfer search symbol "<query>" --agentic --limit 20`
2. If the JSON output contains `"suggested_command"`, run that command, then retry the search.

## Search options

Use these flags when relevant to the user's request:

- `--type class,method,field` — filter by symbol kind (comma-separated; omit for all types)
- `--dependency <GAV>` — restrict to dependencies matching a GAV pattern (glob supported, e.g., `"com.google.*:guava:*"`)
  - Query can be omitted with `--dependency` to list all symbols in that dependency
- `--package <pattern>` — filter by Java package (glob supported, e.g., `"com.google.common.collect.*"`)
  - Query can be omitted with `--package` to list all symbols in matching packages
  - Use `classpath-surfer search pkg --agentic` to discover available package names
- `--access public,protected` — include non-public symbols (default: `public`; use `--access all` for everything)
- `--scope compileClasspath` or `--scope runtimeClasspath` — narrow by configuration scope
- `--offset N` — paginate results (use when `total_matches` exceeds displayed count)

## Smart search behavior

The search mode is auto-detected from the query string:
- Glob patterns (`*`, `?`) with 2+ dots → glob on FQN (e.g., `com.google.*.Immutable*`)
- Glob patterns with fewer dots → glob on simple name (e.g., `Immutable*List`)
- Queries with 2+ dots (no glob) → exact FQN match (e.g., `com.google.common.collect.ImmutableList`)
- Everything else → smart token search:
  - CamelCase tokens are split (e.g., `ImmList` matches `ImmutableList`)
  - Prefix matching is supported (e.g., `Immut` matches `ImmutableList`)
  - Multi-word queries use AND semantics

## Formatting

3. Summarize results as a concise markdown table: FQN, Kind, Signature, Dependency.
   - Use Kotlin signature if available, otherwise Java signature.
4. If `total_matches` exceeds displayed results, note the truncation.
5. Do NOT include raw JSON in the response.

## Troubleshooting

If a CLI command fails unexpectedly (unknown flag, unexpected output format, etc.), run `/manage-index diagnose` to check for CLI/plugin version mismatch.
