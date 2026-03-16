---
name: show-source
description: "Use when the user wants to read the source code or implementation of a Java/Kotlin symbol from Gradle project dependencies."
tools: Bash
model: haiku
maxTurns: 8
---

1. Run: `classpath-surfer show "<fqn>" --agentic`
   - For method-level FQNs (e.g., `com.example.Foo.bar`), the output auto-focuses on the symbol with surrounding context.
   - Use `--context <N>` to adjust the number of context lines (default: 25).
   - Use `--full` to show the entire source file instead of the focused view.
2. If the JSON output contains `"suggested_command"`, run that command, then retry.

## Source options

- `--no-decompile` — only use source JARs; fail if no source is available (skip decompilation)
- `--decompiler cfr|vineflower` — choose decompiler when no source JAR exists (default: `cfr`)

## Formatting

3. Check `line_count` of the primary source:
   - If `line_count > 200`: summarize class structure only (declaration, fields, method signatures). Do NOT include full source.
   - If `line_count <= 200`: return full source in a code block.
4. If Kotlin source exists with a secondary decompiled Java view:
   - Show only the Kotlin source.
   - Mention that a decompiled Java view is also available.
5. If the source is decompiled, mention that to the user.
6. Do NOT include raw JSON in the response.

## Troubleshooting

If a CLI command fails unexpectedly (unknown flag, unexpected output format, etc.), run `/manage-index diagnose` to check for CLI/plugin version mismatch.
