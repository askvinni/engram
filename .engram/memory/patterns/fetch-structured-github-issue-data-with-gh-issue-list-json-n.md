---
title: "Fetch structured GitHub data with gh --json and parse into typed structs"
read_when:
  - "adding a new GitHub API call in github.rs"
  - "fetching issue or PR data from the GitHub API"
  - "deciding between gh CLI and raw API for a new GitHub operation"
tripwires: []
last_updated: "2026-05-18"
source_issues: [10]
---

All GitHub I/O in engram goes through the gh CLI with `--json` flags and parses the output via `serde_json::from_str` into typed structs. This keeps authentication handled by gh (no token management), keeps the code consistent, and means new fields are just added to the struct definition. The `gh` wrapper function in src/github.rs handles error propagation uniformly. Avoid mixing in raw HTTP or GraphQL for REST-accessible data — use GraphQL only when the REST API can't express the query (e.g. finding the PR that closed an issue via CLOSED_EVENT timeline).
