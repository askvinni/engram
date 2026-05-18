---
title: "Globbing a prompt-hooks directory loads README.md as a rule file unless excluded"
read_when:
  - "loading files from a directory that may contain documentation files"
  - "implementing or changing load_prompt_hooks()"
  - "adding a new directory where users place configuration files alongside a README"
tripwires:
  - action: "Reading all .md files from a user-populated directory without filtering"
    warning: "README.md and other documentation files will be injected as rules; always filter by filename convention before loading"
last_updated: "2026-05-18"
source_issues: [26]
---

When loading all `.md` files from a directory as prompt rules or configuration, documentation files like README.md will be included unless explicitly skipped. The README in `.engram/prompt-hooks/` explains the directory's purpose and is not a rule — injecting it into the Claude prompt produces confusing meta-instructions. Filter by checking `e.file_name() != "README.md"` (or a more general documentation filename list) before loading. This applies to any directory where engram writes a README alongside user-provided files. See src/claude.rs:load_prompt_hooks.
