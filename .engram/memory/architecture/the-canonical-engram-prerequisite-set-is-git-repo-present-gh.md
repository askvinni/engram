---
title: "The canonical engram prerequisite set: git repo, gh, gh auth, claude, config.toml, github repo"
read_when:
  - "adding a new check to engram doctor"
  - "setting up engram in a new environment and debugging failures"
  - "deciding what prerequisites a new engram feature should require"
tripwires: []
last_updated: "2026-05-18"
source_issues: [9]
---

Engram depends on exactly six prerequisites: (1) a git repo at or above the working directory, (2) the gh CLI installed, (3) gh authenticated to GitHub, (4) the claude CLI installed, (5) a .engram/config.toml file, and (6) a GitHub repo configured within it. Doctor validates all six in order. New features should require only a subset of these — avoid introducing new external dependencies unless strictly necessary. If a new feature does need a new prerequisite, add it to doctor at the same time so users get a clear error message instead of a cryptic failure.
