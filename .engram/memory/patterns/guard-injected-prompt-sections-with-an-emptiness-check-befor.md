---
title: "Guard optional prompt sections with an emptiness check to avoid empty headers"
read_when:
  - "adding a new optional section to the synthesis prompt"
  - "implementing prompt injection logic where the content may be absent"
  - "building a prompt template with conditionally included sections"
tripwires: []
last_updated: "2026-05-18"
source_issues: [13]
---

When building prompts that include optional sections (e.g. project-specific rules, current memory), always check whether the content is empty before interpolating the section — including the header. An empty `## Project-Specific Rules` header with no content confuses the model and wastes tokens. The pattern is: compute the section string (empty string if no content), and only emit the header + content when the string is non-empty. See src/claude.rs:synthesize_learnings for the hooks_section guard.
