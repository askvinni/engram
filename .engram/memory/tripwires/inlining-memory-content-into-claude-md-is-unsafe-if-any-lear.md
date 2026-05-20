---
title: "Inlining memory content into CLAUDE.md corrupts section boundaries if content contains the end marker"
read_when:
  - "writing memory content into CLAUDE.md"
  - "implementing or changing write_claude_md_section()"
  - "changing the engram section delimiter strings"
tripwires:
  - action: "Inlining raw memory content between engram section markers in CLAUDE.md"
    warning: "If any learning contains the closing marker string (<!-- engram:end -->), it corrupts the section boundary; use @path references instead of inlined content"
  - action: "Using find() instead of rfind() to locate the closing section marker"
    warning: "find() matches the first occurrence, which may be inside inlined content rather than the actual boundary; always use rfind() for the closing marker"
last_updated: "2026-05-18"
source_issues: [8]
---

The engram section in CLAUDE.md is delimited by `<!-- engram:start -->` and `<!-- engram:end -->` markers. If learning content is inlined between these markers and any learning happens to contain the closing marker string literally, the section boundary detection breaks — this happened in production. Two fixes: (1) never inline content, use @path references only, and (2) use `rfind()` not `find()` when locating the closing marker, so it always finds the actual boundary rather than a stray match inside content. See src/memory.rs:write_claude_md_section.
