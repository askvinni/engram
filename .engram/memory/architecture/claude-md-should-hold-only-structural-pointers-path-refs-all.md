---
title: "CLAUDE.md should hold only structural pointers, not inlined memory content"
read_when:
  - "deciding what belongs in CLAUDE.md vs .engram/memory/ files"
  - "implementing or changing write_claude_md_section()"
  - "CLAUDE.md is growing large or mixing routing with content"
tripwires: []
last_updated: "2026-05-18"
source_issues: [8]
---

CLAUDE.md is the agent's entry point, not a content store. It should contain only structural @path references that point agents to the memory system — never inlined learning content. Inlining causes two problems: the file grows unbounded as learnings accumulate, and any learning that contains a structural marker (like the engram section delimiters) corrupts the file's own boundaries. The @path import pattern keeps CLAUDE.md stable and small regardless of how many learnings are added. See src/memory.rs:write_claude_md_section for the implementation.
