---
title: "Use YAML frontmatter to make knowledge files machine-navigable"
read_when:
  - "designing a file format for storing learned knowledge that agents will read"
  - "adding metadata (routing conditions, structured warnings, provenance) to markdown docs"
  - "building a system where both humans and agents consume the same knowledge files"
tripwires: []
last_updated: "2026-05-18"
source_issues: [29]
---

Markdown files intended for AI consumption benefit from a thin YAML frontmatter block that carries structured metadata the agent can parse without reading the body: read_when (routing conditions), tripwires (action/warning pairs), last_updated, and source_issues. This lets an index-builder generate a routing table automatically and lets agents decide relevance before loading full content. The body remains human-readable prose, preserving dual use. The frontmatter schema should be versioned implicitly via slug conventions and category subdirectories so new fields can be added without breaking old parsers.
