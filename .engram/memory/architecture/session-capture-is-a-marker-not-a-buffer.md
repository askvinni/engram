---
type: architecture
title: 'Mid-session observations are ordinary capture files tagged origin: session, not a separate buffer or category'
read_when:
- extending engram capture --session or adding another way for agents to write memory during a session
- implementing compact logic that should treat unreviewed session observations differently from learn output
- tempted to add a session buffer, queue, or dedicated category for agent observations
tripwires:
- action: Adding a separate session-notes category, staging directory, or buffer file for mid-session observations
  warning: 'The design keeps session writes as normal capture files with origin: session in frontmatter. That way read, search, index.md routing, and migrate handle them with no extra work. A separate store would need its own indexing and consolidation path, and the issue explicitly ruled that scope out.'
generated:
  by: engram/0.1.0
  at: 2026-10-19
sources:
- id: '90'
  resource: https://github.com/askvinni/engram/issues/90
---

engram capture --session goes through the same capture::write path as plain capture: same validation, same rendering, same FTS upsert, and the same no-overwrite guard (use --slug on a collision). The only difference is the origin: session key in Frontmatter.extra. No issue or PR is needed, so agents can record a learning mid-task without breaking flow. The marker is how compact recognizes a file as an unreviewed observation that may need merging or pruning, so do not add a distinct category or storage location. The JSON output adds an origin field only when it is set (skip_serializing_if), so plain-capture consumers see no change. Any new agent-facing write path should follow the same approach: reuse capture::write and set a marker in extra, without creating a parallel store.
