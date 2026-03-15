---
name: manage-index
description: "Manage the classpath-surfer symbol index: init, refresh, check status, or clean. Use when the user wants to initialize, update, inspect, or remove the dependency index."
allowed-tools: Bash(classpath-surfer *)
argument-hint: "[action: init|refresh|status|clean]"
disable-model-invocation: true
---

Manage the classpath-surfer index for this project.

Determine the action from "$ARGUMENTS" (default: `refresh`):

- **init**: `classpath-surfer init`
- **refresh**: `classpath-surfer refresh`
- **status**: `classpath-surfer status --agentic`
- **clean**: `classpath-surfer clean`

After the action completes, run `classpath-surfer status --agentic` (unless the action was `clean`) and report the result to the user.
