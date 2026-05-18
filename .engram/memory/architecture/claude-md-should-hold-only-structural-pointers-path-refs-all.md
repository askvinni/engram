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

CLAUDE.md is the agent's entry point, not a content store. It should contain only structural `@path` references — e.g. `@.engram/memory/index.md` — that point agents to the memory system; never inline learning content between the `<!-- engram:start -->` / `<!-- engram:end -->` section markers. Inlining causes three concrete problems: the file grows unbounded as learnings accumulate; any learning that contains the closing marker string literally silently corrupts the section boundary (this happened in production); and CLAUDE.md must be rewritten on every learn run, creating content-drift risk. Claude Code's `@path` import syntax resolves references at load time, so CLAUDE.md stays stable and small regardless of how many learnings accumulate and agents always get current content without rewrites. Only the index.md path needs to be in CLAUDE.md — the index itself references individual topic files. See src/memory.rs:write_claude_md_section for the implementation.
