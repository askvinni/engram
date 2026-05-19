---
title: "Use learn::commit_memory_pr for the git workflow tier between write_memory and run"
read_when:
  - "implementing a batch command that synthesizes learnings for multiple issues into a single branch and PR"
  - "adding a new command that needs git workflow (branch+commit+push+PR) but not full issue lifecycle management"
tripwires:
  - action: "Inlining a git branch+commit+push+PR block in a new learn-adjacent command instead of calling learn::commit_memory_pr"
    warning: "The shared git workflow lives in learn::commit_memory_pr — duplicating it creates drift; call write_memory in the issue loop, then commit_memory_pr once after the loop"
last_updated: "2026-05-19"
source_issues: [69]
---

The learn module now exposes three tiers of increasing scope: write_memory() writes memory files to disk and returns a bool (changed/unchanged), commit_memory_pr() takes those on-disk changes and creates a branch, commits, pushes, and opens a PR, and run() does all of that plus closes the issue and applies the engram-learned label. Batch commands like plan::learn_all and objective::land call write_memory() inside their issue loop and invoke commit_memory_pr() exactly once after the loop, producing one branch and one PR covering all processed issues. Previously the git block was duplicated verbatim in both cmd_learn_all and objective::land; commit_memory_pr is now the canonical single implementation and any new command that creates a learn PR must use it. See src/learn.rs:commit_memory_pr.
