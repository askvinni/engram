---
title: "Compute human-readable issue age without adding a date library dependency"
read_when:
  - "displaying issue age or timestamps in CLI output"
  - "adding date formatting to any engram command"
  - "deciding whether to add a date/time crate dependency"
tripwires: []
last_updated: "2026-05-18"
source_issues: [10]
---

Human-readable ages ("today", "1 day ago", "N days ago") can be computed with a small inline Gregorian day-count formula using only the standard library — no chrono or time crate needed. The formula converts a YYYY-MM-DD string to days-since-epoch, subtracts from the current epoch day, and formats the result. This avoids a dependency for a feature that is genuinely simple. See src/main.rs:days_ago and days_from_ymd for the implementation.
