---
name: engram-learn
description: Use this skill when the user asks to "run engram learn", "synthesize learnings", "run engram land", "extract memory from issue", "what did we learn from #N", or is performing the post-PR memory workflow. Provides guidance on the learn/land workflow, what gets written to memory, and how to review synthesized learnings.
version: 1.0.0
allowed-tools: Bash(engram guide*)
---

# engram-learn skill

Run `engram guide learn` and follow the workflow it prints. It is served by the installed engram binary, so it is always current.

Arguments: **$ARGUMENTS**. Where the guide refers to the arguments passed to this skill, use these; if there are none, follow the guide's instruction for that case.

If `engram guide` is not a known command, the installed engram is too old: tell the user to upgrade engram (then re-run `engram init`) and stop.
