---
title: "engram validation emits stderr warnings but proceeds — bumpers, not hard blocks"
read_when:
  - "adding input validation to any engram command"
  - "deciding whether a validation failure in cmd_plan or a similar command should abort or warn"
tripwires: []
last_updated: "2026-05-18"
source_issues: [49]
---

The established engram pattern for user-facing validation is to print a warning to stderr naming the problem, then proceed with the operation regardless. cmd_plan() uses this for missing plan sections: it calls missing_plan_sections(), prints each missing header name to stderr, and then calls gh issue create unconditionally. The rationale is that blocking on validation failures frustrates users and encourages workarounds, whereas a visible stderr warning preserves discoverability without preventing progress. Any new validation added to engram commands should follow the same pattern unless there is a hard technical reason — such as a malformed required argument — that makes proceeding impossible. See src/main.rs:cmd_plan.
