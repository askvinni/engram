---
name: engram-memory
description: Use this skill when the user asks to "check what engram knows", "read the memory", "run engram compact", "prune memory files", "is this memory file worth keeping", "review learnings", or is navigating, evaluating, or compacting the .engram/memory/ directory. Provides guidance on the memory layout, category semantics, compaction workflow, and manual maintenance.
version: 1.0.0
allowed-tools: Bash(engram guide*)
---

# engram-memory skill

Run `engram guide memory` and follow the workflow it prints. It is served by the installed engram binary, so it is always current.

Arguments: **$ARGUMENTS**. Where the guide refers to the arguments passed to this skill, use these; if there are none, follow the guide's instruction for that case.

If `engram guide` is not a known command, the installed engram is too old: tell the user to upgrade engram (then re-run `engram init`) and stop.
