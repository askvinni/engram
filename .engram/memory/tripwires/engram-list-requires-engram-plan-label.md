---
title: "engram list only surfaces issues that carry the engram-plan label"
read_when:
  - "creating a GitHub issue that should appear in engram list output"
  - "debugging why an expected plan issue does not appear in engram list"
tripwires:
  - action: "Creating a GitHub issue by hand (outside of engram plan) to track engram work"
    warning: "list_open_plans filters with --label engram-plan; issues without that label are invisible to engram list no matter their title or state"
last_updated: "2026-05-18"
source_issues: [10]
---

list_open_plans in src/github.rs passes --label engram-plan to gh issue list, so only issues carrying that exact label are returned. This constraint is invisible at issue-creation time: if you open an issue manually or via another tool without adding the label, engram list will silently produce an empty list with no diagnostic. The label must be added at creation time; there is no retroactive scan or fallback. See src/github.rs:list_open_plans for the filter.
