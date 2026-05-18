---
title: "Define health checks as (&str, Box<dyn Fn() -> bool>) tuples for declarative expansion"
read_when:
  - "adding a new check to engram doctor"
  - "implementing a diagnostics or validation command"
  - "deciding how to structure a set of independent boolean checks"
tripwires: []
last_updated: "2026-05-18"
source_issues: [9]
---

A slice of `(&str, Box<dyn Fn() -> bool>)` tuples separates the check declarations from the display/exit logic cleanly. Adding a new check is a one-liner in the slice; the display loop and bail logic never change. The label string doubles as both user-facing output and a test identifier. This pattern also makes it easy to collect all results before bailing — iterate the slice, run each check, accumulate failures, then exit once. See src/main.rs:cmd_doctor for the implementation.
