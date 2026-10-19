---
type: pattern
title: Every GitHub-only command must get owner/name from config::resolve_repo, never from its own gh repo view call
read_when:
- adding a new engram subcommand that calls gh or the GitHub API
- adding a gh step to engram init, capture, or another command that must work in local-only repos
- debugging an engram command that crashes or hangs in a repo with no origin remote or no gh installed
tripwires:
- action: Writing a module-local resolve_repo or calling Command::new("gh").args(["repo", "view", ...]) directly
  warning: Use config::resolve_repo for GitHub-only commands. It fails with a consistent 'this command requires GitHub' error. Use config::infer_repo, which returns an Option, for optional gh steps. A private copy, like the one compact.rs used to have, shells out to gh in local-only repos and gives a vague error or a crash.
- action: Adding an optional gh step (labels, repo inference) to init or capture without gating it
  warning: Check config::gh_available() and config::has_remote() first and skip the step when either is false. init and capture are required to succeed with no origin remote and no gh installed.
generated:
  by: engram/0.1.0
  at: 2026-10-19
sources:
- id: '94'
  resource: https://github.com/askvinni/engram/issues/94
---

engram splits commands into two tiers. init and capture must work with no GitHub at all. plan, learn, objective, compact, and memory migrate are GitHub-only, and in a local-only repo they must fail loudly instead of degrading silently. src/config.rs holds both halves. infer_repo is best-effort and returns None without running gh when there is no remote or gh is missing. resolve_repo is the strict entry point. It bails with the REQUIRES_GITHUB prefix and a specific reason, and a missing gh is reported first even when github.repo is configured. A configured github.repo removes the need for a git remote but not the need for gh. compact.rs used to carry a duplicate resolve_repo that called gh unconditionally; it was removed so there is only one place where the repo is resolved and one place where the error is worded. Any future GitHub-backed command should call config::resolve_repo at the top. Any optional gh step should copy the gh_available gate, which follows the same pattern as kata::kata_available.
