---
title: "Identifier-resolution commands use 0/1/N outcome contract: nothing/content/candidate-list"
read_when:
  - "implementing a new engram subcommand that resolves a user-provided name, slug, or path to a file or record"
  - "designing the error behavior for any engram lookup command that might match zero or multiple results"
tripwires:
  - action: "Implementing a file-resolver that silently picks the first match when multiple files share the same stem"
    warning: "The established engram contract is: N>1 matches must list all candidates with relative paths and exit non-zero — silent picks are especially harmful in --agent mode where the caller expects deterministic output; see src/main.rs:cmd_read"
last_updated: "2026-09-08"
source_issues: [96]
---

See src/main.rs:cmd_read for the reference implementation. In --agent mode, failure paths must prefix output with `ERR` so callers can parse without inspecting exit codes. The N>1 path is critical — a silent pick would cause the calling agent to act on an arbitrary file with no indication of ambiguity.
