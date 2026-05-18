---
title: "Store repo-specific prompt customization as committed .md files in .engram/prompt-hooks/"
read_when:
  - "allowing per-repo AI behavior customization"
  - "implementing or changing load_prompt_hooks()"
  - "designing engram extensibility for different team workflows"
tripwires: []
last_updated: "2026-05-18"
source_issues: [13]
---

Repo-specific rules for how engram classifies learnings (e.g. "always classify Rust lifetime errors as tripwires", "use pytest patterns for testing entries") belong in `.engram/prompt-hooks/` as committed Markdown files. Loading them alphabetically and injecting as a named prompt section means AI classification behavior is tunable without code changes and automatically shared across the team via git. The directory is created by `engram init` with a README explaining the contract. Files are loaded alphabetically so ordering is deterministic and teams can prefix filenames to control priority. See src/claude.rs:load_prompt_hooks.
