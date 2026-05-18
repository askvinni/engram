---
title: "Resolve the GitHub repo by checking config first, then falling back to gh CLI"
read_when:
  - "adding a new engram command that needs the GitHub repo name"
  - "implementing or changing repo resolution logic"
  - "deciding whether to require explicit config or auto-detect the repo"
tripwires: []
last_updated: "2026-05-18"
source_issues: [10]
---

Every command that touches GitHub needs the `owner/repo` string. The two-step resolution — check `.engram/config.toml` first, fall back to `gh repo view --json nameWithOwner` — covers both explicitly configured repos and repos where the user hasn't run init yet. Never skip the fallback (requiring config) or skip the config check (always shelling out): the config check avoids a subprocess in the common case, and the fallback avoids a hard failure when the repo is unambiguous from git context. See src/main.rs:infer_repo and each cmd_* function for the established pattern.
