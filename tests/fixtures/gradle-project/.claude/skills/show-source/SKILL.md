---
name: show-source
description: Show source code of a Java/Kotlin symbol from project dependencies. Use when the user wants to read the implementation of an external library class or method.
allowed-tools: Bash(classpath-surfer *)
argument-hint: "[fully-qualified-name]"
---

Show the source code for "$ARGUMENTS".

Run: `classpath-surfer show "$ARGUMENTS" --agentic`

If the output contains "Index is stale" or "No index found", run:
1. `classpath-surfer index refresh`
2. `classpath-surfer show "$ARGUMENTS" --agentic`

If it returns decompiled source, mention that to the user.
Display the source code in a Java code block.
