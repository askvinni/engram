---
title: "Store machine-readable state in hidden HTML comments within GitHub issue bodies"
read_when:
  - "attaching structured data to a GitHub issue that must survive gh issue edit --body round-trips"
  - "designing a command that reads and writes structured state from a GitHub issue body"
  - "implementing a new engram command that updates issue body content programmatically"
tripwires: []
last_updated: "2026-05-19"
source_issues: [54]
---

The engram:nodes pattern stores ObjectiveNode JSON inside an HTML comment (`<!-- engram:nodes [...] -->`) embedded in the issue body. GitHub renders HTML comments as invisible, so the raw JSON is hidden from users but preserved verbatim through `gh issue edit --body` round-trips — the gh CLI passes the body string literally, leaving comments intact. A human-readable Markdown table is rendered above the comment and regenerated from the JSON on every write, keeping display and canonical storage decoupled so the table format can change without touching the data. Any future command that needs durable structured state attached to a GitHub issue should use the same pattern: pick a unique namespace string, embed JSON as `<!-- engram:namespace JSON -->`, and always regenerate display representations from the JSON rather than parsing the table. See src/objective.rs NODES_MARKER_START/NODES_MARKER_END and parse_nodes_from_comment/write_nodes_to_comment.
