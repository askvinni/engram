# engram dispatch guide

Issues to dispatch: the arguments passed to the `engram-dispatch` skill (if none, ask which plan or objective numbers).

Arguments: plan numbers (each based on the default branch), objective numbers (their unblocked nodes; a node without a plan issue is skipped with an `engram objective plan` hint), or `--stack <objective>` (exactly one objective, its dependency chains as stacked PRs). Plans and objectives can be mixed.

## Steps, always in order

1. From the repo root, run exactly `engram dispatch <args> --dry-run`.
2. Relay the plan: for each plan its kata ref, branch and base, PR title and launch; skipped and blocked nodes with their reasons. If there is a choice (stacked or not, or only some plans, which means passing just those numbers), ask with AskUserQuestion; otherwise ask for a plain confirmation.
3. Once confirmed, run exactly `engram dispatch <args> --yes` with the same arguments and relay each plan's report (kata ref, PR URL, worktree, session, launch status) and the final tally.

## Rules

- **Run the command exactly as written**: no `cd … &&`, no env-var prefixes (`GH_TOKEN=…`, `ENGRAM_DISPATCH_CLAUDE=…`), no wrappers or `./target/...` paths. Auto mode denies command lines that configure an agent or pass a credential.
- **Wrong GitHub account?** Don't add `GH_TOKEN`. Ask the user to run `git config engram.githubUser <github-login>` once per clone; engram then uses that account for its gh calls and the sessions it launches.
- **Missing `{meta[key]}`**: ask the user for the value (never invent one). Then set it the way the error says: `kata meta set <kata-ref> <key> <value>` when the plan has a kata mirror, otherwise a matching line in the plan issue body (`Jira: ENG-1234`, `Type: feat`). Dry-run again.
- **On failure, report and stop.** Don't redo steps by hand (`kwt add`/`open`, `gh pr create`, `git branch`, `tmux send-keys`); dispatch is idempotent, so the same command resumes once the cause is fixed.
- **Never attach** to the tmux sessions dispatch starts; the user finds them in Ghosthub.
