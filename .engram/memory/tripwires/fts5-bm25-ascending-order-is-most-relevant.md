---
title: "FTS5 bm25() returns negative scores — ORDER BY rank ASC gives most relevant first"
read_when:
  - "implementing or changing a full-text search query against memory_fts in index.rs"
  - "adding a new FTS5 search function that ranks results by relevance"
tripwires:
  - action: "Adding ORDER BY rank DESC to a query that aliases bm25(memory_fts) AS rank"
    warning: "FTS5 bm25() returns negative values; more relevant documents score more negative, so DESC order returns least relevant first — omit the direction or use ASC to get most relevant first"
last_updated: "2026-09-08"
source_issues: [92]
---

SQLite FTS5's bm25() function returns negative floating-point scores: a highly relevant match gets something like -5.2 while a weak match gets -0.3. This means ORDER BY rank with no direction (ASC by default) correctly surfaces the most relevant results first, but ORDER BY rank DESC — the intuitive 'sort by score descending' choice — silently inverts the ranking and returns least relevant first with no error. The search() function in src/index.rs uses this correctly with a bare ORDER BY rank. Any future FTS5 query that wants relevance-ordered output must follow the same pattern; getting it wrong produces valid-looking results in the wrong order.
