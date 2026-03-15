---
name: find-symbol
description: Search for Java/Kotlin class, method, or field symbols in Gradle project dependencies. Use when the user needs to find API signatures, class locations, or method definitions from external libraries.
tools: Bash
model: haiku
---

1. Run: `classpath-surfer search "<query>" --agentic --limit 20`
2. If the JSON output contains `"suggested_command"`, run that command, then retry the search.
3. By default, only `public` symbols are shown. To include other access levels, add `--access public,protected` (or `--access all`).
4. Summarize results as a concise markdown table: FQN, Kind, Signature, Dependency.
   - Use Kotlin signature if available, otherwise Java signature.
5. If `total_matches` exceeds displayed results, note the truncation.
6. Do NOT include raw JSON in the response.
