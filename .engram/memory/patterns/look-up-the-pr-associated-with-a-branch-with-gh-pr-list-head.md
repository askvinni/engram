---
title: "Look up the PR associated with a branch using gh pr list --head"
read_when:
  - "finding the PR for the current branch"
  - "implementing status command or any command that needs the branch's PR"
  - "deciding how to look up a PR without knowing its number"
tripwires: []
last_updated: "2026-05-18"
source_issues: [12]
---

`gh pr list --repo <repo> --head <branch> --limit 1 --json number,title,body,state` returns an empty JSON array when no PR exists for the branch, making it natural to model as `Option<PullRequest>`. This is the correct lookup for "what PR is open/closed for this branch" — distinct from find_linked_pr which answers "what PR closed this issue". Use --limit 1 since at most one open PR can exist per branch, and include state in the JSON fields if you need to filter by merged/open. See src/github.rs:find_pr_for_branch.
