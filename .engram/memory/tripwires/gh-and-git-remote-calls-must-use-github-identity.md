---
type: tripwire
title: Spawn GitHub-facing gh and origin-facing git through config::gh_command/with_github_identity, never bare Command::new("gh")
read_when:
- adding a new gh subprocess call anywhere in engram (github.rs, dispatch.rs, kata.rs, objective code)
- adding a git fetch, push or ls-remote against origin from engram code
- debugging gh or git returning 'not found' or permission errors on a repo the user can reach with another GitHub account
tripwires:
- action: Calling Command::new("gh") or a git remote command directly in a new code path
  warning: That skips the engram.githubUser identity, so on multi-account machines the call runs as the active (wrong) account. Use config::gh_command()/with_github_identity, or dispatch's git_remote for git.
- action: Falling back to the active gh account when `gh auth token --user <login>` fails
  warning: Fail with a `gh auth login` hint instead. A silent fallback acts on the wrong account. An existing GH_TOKEN or GITHUB_TOKEN wins, and gh auth token must not be called then (token_user_to_resolve).
generated:
  by: engram/0.1.0
  at: 2026-10-27
sources:
- id: '154'
  resource: https://github.com/askvinni/engram/issues/154
---

Users with several GitHub accounts set `git config engram.githubUser <login>` per clone. It lives in git config, not .engram/config.toml, because the identity belongs to the person, not the repo. engram resolves `gh auth token --user <login>` once and passes the result as GH_TOKEN, and only GH_TOKEN. It never runs `gh auth switch`, which would change the user's global state. This only works if every subprocess that talks to GitHub goes through the shared helpers. That includes core gh calls, ensure_label, the gh-stack extension and dispatch's git fetch/push/ls-remote (git_remote). One bare spawn brings back the 'wrong account' failure for that command only, which is hard to diagnose. engram doctor reports why gh auth failed, so it is the first thing to check. See src/config.rs:gh_command and with_github_identity.
