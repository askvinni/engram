# patterns

- Use @path imports in CLAUDE.md to reference .engram/memory/*.md files instead of inlining content — keeps CLAUDE.md small and decoupled from memory growth. _(from #8)
- When building @path reference lists, filter to only files that actually exist and sort by path for deterministic output. _(from #8)
- In doctor-style commands, collect all check results before bailing — print ✓/✗ for every check so users see the full diagnostic rather than stopping at the first failure. _(from #9)
- Define health checks as a slice of (&str, Box<dyn Fn() -> bool>) tuples so new checks can be added declaratively without touching the display/exit logic. _(from #9)
- Fetch structured GitHub issue data with `gh issue list --json number,title,createdAt` and parse via `serde_json::from_str` into typed structs — keeps all GitHub I/O consistent with the rest of the codebase. _(from #10)
- Compute human-readable issue age ("today", "1 day ago", "N days ago") with a small inline Gregorian day-count formula rather than adding a date library dependency. _(from #10)
