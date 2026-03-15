---
name: list-deps
description: "Use when the user wants to explore, browse, or filter project dependencies — list indexed libraries, check symbol counts, or find a specific dependency by GAV pattern."
tools: Bash
model: haiku
maxTurns: 5
---

1. Run: `classpath-surfer deps --agentic`
2. If the JSON output contains `"suggested_command"`, run that command, then retry.

## Filter options

- `--filter <GAV>` — filter dependencies by GAV pattern (glob supported, e.g., `"com.google.*:*"`)
- `--scope compileClasspath` or `--scope runtimeClasspath` — filter by configuration scope
- `--limit N` — maximum number of results (default: 50)
- `--offset N` — skip results for pagination

## Formatting

3. Summarize results as a concise markdown table: GAV, Scope, Symbol Count.
4. If the total exceeds displayed results, note the truncation and suggest `--offset` for more.
5. Do NOT include raw JSON in the response.
