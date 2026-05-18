---
title: "Compose higher-level workflow commands by calling existing functions internally"
read_when:
  - "implementing a new engram command that orchestrates existing commands"
  - "deciding whether to duplicate logic or call existing command functions"
  - "adding a command that is a superset of another command"
tripwires: []
last_updated: "2026-05-18"
source_issues: [11]
---

When a new command is a workflow superset of existing commands (e.g. land = learn + close + cleanup), call the existing implementation functions directly rather than duplicating the logic. This means the composed command inherits all bug fixes and new features added to the underlying commands for free. The tradeoff is that the composed command's output interleaves steps from multiple sources — which is desirable since it shows the full workflow. See src/main.rs:cmd_land calling learn::run() as the established pattern.
