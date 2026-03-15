---
name: show-source
description: Show source code of a Java/Kotlin symbol from Gradle project dependencies. Use when the user wants to read the implementation of an external library class or method.
tools: Bash
model: haiku
---

1. Run: `classpath-surfer show "<fqn>" --agentic`
2. If the JSON output contains `"suggested_command"`, run that command, then retry.
3. Check `line_count` of the primary source:
   - If `line_count > 200`: summarize class structure only (declaration, fields, method signatures). Do NOT include full source.
   - If `line_count <= 200`: return full source in a code block.
4. If Kotlin source exists with a secondary decompiled Java view:
   - Show only the Kotlin source.
   - Mention that a decompiled Java view is also available.
5. If the source is decompiled, mention that to the user.
6. Do NOT include raw JSON in the response.
