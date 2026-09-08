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

cmd_read establishes the canonical three-branch contract for any engram command that resolves a user-typed identifier to a file: 0 matches exits non-zero with a one-line 'not found' message (prefixed ERR in --agent mode); 1 match prints content and exits 0; 2+ matches prints all candidate paths relative to the memory root and exits non-zero. The N>1 path is especially important in --agent mode — a silent pick would cause the calling agent to act on an arbitrary file with no indication something was wrong, and interactive prompting is impossible in non-interactive contexts. Any future resolver subcommand (resolve-by-tag, read-by-title, etc.) must follow the same three-branch shape. See src/main.rs:cmd_read for the reference implementation.
