---
name: engram-dispatch
description: Use this skill when the user asks to "dispatch" an engram plan or objective, "dispatch #N", "dispatch the unblocked nodes", "stack an objective", "spawn worktrees for these plans", or "start Claude sessions on these plan issues". Runs `engram dispatch`, which gives each plan its own branch, draft PR, kwt worktree and background Claude session in auto mode.
version: 0.1.0
allowed-tools: Bash(engram dispatch:*)
---

# engram-dispatch skill

Issues to dispatch: **$ARGUMENTS** (if empty, ask which plan or objective numbers).

Arguments: plan numbers (each based on the default branch), objective numbers (their unblocked nodes; missing plan issues are created first), or `--stack <objective>` (exactly one objective, its dependency chains as stacked PRs). Plans and objectives can be mixed.

## Steps, always in order

1. From the repo root, run exactly `engram dispatch <args> --dry-run`.
2. Relay the plan: for each plan its kata ref, branch and base, PR title and launch; skipped and blocked nodes with their reasons. If there is a choice (stacked or not, or only some plans, which means passing just those numbers), ask with AskUserQuestion; otherwise ask for a plain confirmation.
3. Once confirmed, run exactly `engram dispatch <args> --yes` with the same arguments and relay the summary (kata refs, PR URLs, worktrees, sessions, launch status).

## Rules

- **Run the command exactly as written**: no `cd … &&`, no env-var prefixes (`GH_TOKEN=…`, `ENGRAM_DISPATCH_CLAUDE=…`), no wrappers or `./target/...` paths. The allowed rule `Bash(engram dispatch:*)` only matches commands that start with `engram dispatch`.
- **Wrong GitHub account?** Don't add `GH_TOKEN`. Ask the user to run `git config engram.githubUser <github-login>` once per clone; engram then uses that account for its gh calls and the sessions it launches.
- **Missing `{meta[key]}`**: ask the user for the value (never invent one), set it with the `kata meta set <kata-ref> <key> <value>` command the error prints, and dry-run again.
- **On failure, report and stop.** Don't redo steps by hand (`kwt add`/`open`, `gh pr create`, `git branch`, `tmux send-keys`); dispatch is idempotent, so the same command resumes once the cause is fixed.
- **Never attach** to the tmux sessions dispatch starts; the user finds them in Ghosthub.
