---
title: "Look up the PR associated with a branch with `gh pr list --…"
read_when:
  - "(migrated — add read_when conditions)"
tripwires: []
last_updated: "2026-05-18"
source_issues: [12]
---

Look up the PR associated with a branch with `gh pr list --head <branch> --limit 1 --json ...` — returns an empty array when no PR exists, making it natural to model as `Option<PullRequest>`.
