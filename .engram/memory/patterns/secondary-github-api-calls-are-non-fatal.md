---
title: "Treat secondary GitHub API calls (sub-issue linking, label decoration) as non-fatal — warn and continue"
read_when:
  - "adding a secondary GitHub API call after a primary issue or PR creation"
  - "deciding whether a GitHub helper step failure should abort an engram command"
tripwires: []
last_updated: "2026-05-19"
source_issues: [59]
---

The established engram pattern for GitHub API calls that enrich a result but are not required for correctness is to catch the error, print it to stderr, and return success. The issue spec for add_sub_issue explicitly designated linking failures as 'non-fatal — warn on stderr but do not fail the overall command', and create_plan_for_node in src/objective.rs implements exactly that: `if let Err(e) = github::add_sub_issue(...) { eprintln!("warning: ..."); }` with no early return. The rationale is that a GitHub API hiccup, missing permission, or feature unavailability on a convenience step (sub-issue linking, label decoration, status comment) should not prevent the user's primary workflow from completing. Applying this pattern requires consciously deciding at design time which steps are 'core invariants' versus 'convenience' — then encoding that decision visibly so future readers understand the non-fatal choice was deliberate. See src/objective.rs:create_plan_for_node.
