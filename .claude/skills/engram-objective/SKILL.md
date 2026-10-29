---
name: engram-objective
description: Use this skill when the user asks to "create an objective", "open an objective issue", "write an objective for X", "use engram objective", "start a new multi-node goal", or is about to begin work that will be tracked as an engram-objective GitHub issue. Provides a guided six-step flow for creating well-formed objective issues that work with --all-unblocked dispatch and auto-close mechanics.
version: 1.0.0
allowed-tools: Bash(engram guide*), Bash(engram objective*)
---

# engram-objective skill

Run `engram guide objective` and follow the workflow it prints. It is served by the installed engram binary, so it is always current.

Arguments: **$ARGUMENTS**. Where the guide refers to the arguments passed to this skill, use these; if there are none, follow the guide's instruction for that case.

If `engram guide` is not a known command, the installed engram is too old: tell the user to upgrade engram (then re-run `engram init`) and stop.
