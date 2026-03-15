---
name: refresh-deps
description: Re-extract and re-index project dependencies for symbol search. Use when dependencies have changed or the index is stale.
allowed-tools: Bash(classpath-surfer *)
---

Refresh the dependency index for this project.

Run:
1. `classpath-surfer refresh`
2. `classpath-surfer status --agentic`

Report the status to the user when done.
