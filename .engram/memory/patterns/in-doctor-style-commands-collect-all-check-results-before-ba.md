---
title: "In doctor commands, collect all results before bailing to show the full diagnostic"
read_when:
  - "adding a new check to engram doctor"
  - "implementing any validation command that runs multiple independent checks"
  - "deciding when to bail vs continue in a multi-check sequence"
tripwires: []
last_updated: "2026-05-18"
source_issues: [9]
---

When running multiple independent health checks, never bail on the first failure — run all checks, print ✓/✗ for each, then exit with an error if any failed. Users who are setting up engram for the first time often have multiple missing prerequisites; stopping at the first failure forces them into a guess-install-retry loop. Collecting all results upfront gives them a complete to-do list. This is only appropriate for checks that are truly independent — don't run check N if check N-1 is a hard prerequisite for N's validity.
