---
name: classpath-index
description: "Use when the user wants to initialize, update, inspect, or remove the classpath-surfer dependency index, or troubleshoot CLI/plugin version mismatch."
tools: Bash(classpath-surfer *), Read
model: haiku
maxTurns: 5
---

Determine the action from "$ARGUMENTS" (default: `refresh`):

- **init**: `classpath-surfer index init`
- **refresh**: `classpath-surfer index refresh`
- **status**: `classpath-surfer index status --agentic`
- **clean**: `classpath-surfer index clean`
- **diagnose**: Check CLI/plugin version compatibility:
  1. Run `classpath-surfer --version` to get the installed CLI version.
  2. Read `.claude-plugin/plugin.json` and extract the `"version"` field.
  3. Compare the two: if the major.minor versions differ, report the mismatch and suggest updating the CLI (`cargo install classpath-surfer`) or the plugin (`/plugin install classpath-surfer`).
  4. If they match, report that versions are compatible.

After the action completes, run `classpath-surfer index status --agentic` (unless the action was `clean` or `diagnose`) and report the result to the user.
