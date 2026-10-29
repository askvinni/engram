---
name: engram-plan
description: Use this skill when the user asks to "create a plan", "open a plan issue", "write a plan for X", "use engram plan", "start a new feature", or is about to begin work that will be tracked as an engram-plan GitHub issue. Provides guidance on writing effective engram plan issues that produce useful learnings later.
version: 1.0.0
allowed-tools: Bash(engram guide*), Bash(engram plan*)
---

# engram-plan skill

Run `engram guide plan` and follow the workflow it prints. It is served by the installed engram binary, so it is always current.

Arguments: **$ARGUMENTS**. Where the guide refers to the arguments passed to this skill, use these; if there are none, follow the guide's instruction for that case.

If `engram guide` is not a known command, the installed engram is too old: tell the user to upgrade engram (then re-run `engram init`) and stop.
