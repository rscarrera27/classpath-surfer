---
name: find-symbol
description: Search for a Java/Kotlin class, method, or field in project dependencies. Use when the user needs to find API signatures, class locations, or method definitions from external libraries.
allowed-tools: Bash(classpath-surfer *)
argument-hint: "[symbol-name]"
---

Search for the symbol "$ARGUMENTS" in the project's dependency index.

Run: `classpath-surfer search "$ARGUMENTS" --agentic --limit 20`

If the output contains "Index is stale" or "No index found", run:
1. `classpath-surfer refresh`
2. `classpath-surfer search "$ARGUMENTS" --agentic --limit 20`

Present results as a markdown table with columns: Symbol, Kind, Signature, Dependency.
If the user wants to see the source code of a specific result, use /show-source.
