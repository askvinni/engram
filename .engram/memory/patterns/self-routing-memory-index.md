---
title: "Build a routing index so agents load only relevant memory files"
read_when:
  - "designing a memory or knowledge system for AI agents"
  - "deciding how CLAUDE.md or a context entry-point should reference stored knowledge"
  - "memory files are growing and loading all of them on every invocation is wasteful"
tripwires: []
last_updated: "2026-05-18"
source_issues: [29]
---

Loading every memory file on every agent invocation bloats context and dilutes relevance. Instead, maintain an auto-generated index (e.g. index.md) that acts as a routing table: each row names a file, its title, and the conditions under which an agent should load it. The entry-point file (CLAUDE.md or equivalent) references only the index — one line — and agents self-route by reading the table and fetching only files whose read_when conditions match the current task. This keeps context slim regardless of how many topic files accumulate, and the index regenerates deterministically from the topic files themselves so it never drifts.
