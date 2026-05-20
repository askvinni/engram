---
title: "engram doctor's claude check verifies installation only, not non-interactive usability"
read_when:
  - "debugging why engram learn fails after engram doctor reports all checks passing"
  - "adding a new check to cmd_doctor that exercises a dependency beyond binary presence"
tripwires:
  - action: "Trusting engram doctor all-green output as confirmation that claude -p will work correctly"
    warning: "doctor only runs `claude --version`; it does not verify that claude -p can be invoked non-interactively from temp_dir — learn can still fail with agent-mode behaviour even when doctor passes"
last_updated: "2026-05-18"
source_issues: [9]
---

cmd_doctor checks that the claude binary is present via `claude --version`, but does not exercise the non-interactive code path that engram learn actually uses. The critical requirement — that claude -p must be invoked from std::env::temp_dir() to avoid loading repo CLAUDE.md as agent context — is invisible to doctor. A user whose environment passes all six doctor checks can still see learn fail with Claude taking actions instead of returning JSON. When extending doctor with new checks, verify the actual operation mode required (e.g. run a no-op claude -p call from temp_dir), not just binary presence. See src/main.rs:cmd_doctor for the current check list and src/claude.rs:synthesize_learnings for the temp_dir requirement.
