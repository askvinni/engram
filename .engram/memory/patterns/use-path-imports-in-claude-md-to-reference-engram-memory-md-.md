---
title: "Use @path imports in CLAUDE.md to reference memory files instead of inlining content"
read_when:
  - "updating write_claude_md_section()"
  - "deciding how CLAUDE.md should reference the memory system"
  - "CLAUDE.md is getting large or contains stale learning content"
tripwires: []
last_updated: "2026-05-18"
source_issues: [8]
---

Claude Code's `@path` import syntax resolves file references at load time. Using `@.engram/memory/index.md` in CLAUDE.md means the agent gets the current index content on every invocation without CLAUDE.md needing to be rewritten each time memory changes. The alternative — inlining content — requires rewriting CLAUDE.md on every learn run and risks content drift. Only the index.md path needs to be in CLAUDE.md; the index itself references individual topic files. See src/memory.rs:write_claude_md_section.
