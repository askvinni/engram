---
title: "Bypass the gh() helper and use Command::new("gh").output() when a wrapper must be idempotent"
read_when:
  - "adding a gh CLI wrapper in github.rs that must treat one specific failure mode as success rather than an error"
  - "implementing an idempotent GitHub operation that should no-op if the target is already in the desired state"
tripwires:
  - action: "Using the gh() helper for a command that needs to distinguish 'already closed' or 'already exists' from other error kinds"
    warning: "gh() treats all non-zero exits uniformly with no stderr access — use Command::new("gh").output() directly, check output.status.success(), then inspect String::from_utf8_lossy(&output.stderr) for the specific message before deciding whether to return Ok(()) or bail"
last_updated: "2026-05-19"
source_issues: [60]
---

The gh() helper in src/github.rs provides a clean call-and-check interface where any non-zero exit becomes an opaque anyhow error — callers have no way to inspect the stderr content and distinguish one failure kind from another. When a wrapper must be idempotent — close_issue must succeed silently if the issue is already closed — you must drop down to Command::new("gh").output() directly, check output.status.success(), and match String::from_utf8_lossy(&output.stderr) for the specific condition (e.g. stderr.contains("already closed")) before deciding to return Ok(()) or bail. Catching the generic error at the caller and trying to pattern-match it there is worse because anyhow error messages are unstable and the caller has no structured way to distinguish this condition from a permission error or a network failure. This applies to any future wrapper that needs to treat one particular gh CLI failure mode as a deliberate no-op. See src/github.rs:close_issue.
