---
name: engram-dispatch
description: Use this skill when the user asks to "dispatch" an engram plan or objective, "dispatch #N", "dispatch the unblocked nodes", "stack an objective", "spawn worktrees for these plans", or "start Claude sessions on these plan issues". Runs `engram dispatch`, which gives each plan its own branch, draft PR, kwt worktree and background Claude session in auto mode.
version: 1.0.0
allowed-tools: Bash(engram guide*), Bash(engram dispatch:*)
---

# engram-dispatch skill

Run `engram guide dispatch` and follow the workflow it prints. It is served by the installed engram binary, so it is always current.

Arguments: **$ARGUMENTS**. Where the guide refers to the arguments passed to this skill, use these; if there are none, follow the guide's instruction for that case.

If `engram guide` is not a known command, the installed engram is too old: tell the user to upgrade engram (then re-run `engram init`) and stop.
