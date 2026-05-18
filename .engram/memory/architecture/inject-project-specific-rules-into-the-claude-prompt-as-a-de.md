---
title: "Inject project-specific prompt rules as a named section, not raw appended text"
read_when:
  - "adding user-configurable prompt customization to a Claude invocation"
  - "implementing or changing load_prompt_hooks() or synthesize_learnings()"
  - "designing how per-repo AI behavior overrides interact with the base prompt"
tripwires: []
last_updated: "2026-05-18"
source_issues: [13]
---

When injecting optional customization into a Claude prompt, wrap it in a clearly named section header (e.g. `## Project-Specific Rules`) rather than appending raw text. A named section makes the injection visible and auditable in logs, easy to conditionally omit when empty (preventing a stray header), and easy to strip in tests that want to exercise the base prompt only. The section must be guarded by an emptiness check so it disappears entirely when no hooks are defined — an empty header is worse than no header. See src/claude.rs:synthesize_learnings for the implementation.
