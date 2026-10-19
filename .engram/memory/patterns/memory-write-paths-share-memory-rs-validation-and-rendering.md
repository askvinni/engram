---
type: pattern
title: Every memory write path must reuse memory::validate_category, Frontmatter, and render instead of building its own
read_when:
- adding a new command that creates memory files without going through learn
- changing the allowed memory categories or the OKF frontmatter fields
tripwires:
- action: Hard-coding a category list or building frontmatter with format! in a new memory-writing command
  warning: Use memory::validate_category, memory::Frontmatter::new plus touch(), and memory::render. A separate list or hand-built YAML drifts from what learn writes, and read, search, and migrate stop parsing those files consistently.
generated:
  by: engram/0.1.0
  at: 2026-10-19
sources:
- id: '86'
  resource: https://github.com/askvinni/engram/issues/86
---

engram now has several ways to write memory files: learn synthesizes them through Claude, and capture writes caller-supplied content directly. Their outputs must be indistinguishable, so that read, search, index.md routing, and memory migrate treat them the same. capture gets there by sending all validation and serialization through memory.rs: validate_category is the single allowed-category check, Frontmatter::new derives the kind from the category, touch() sets the generated stamp, and render serializes the file. Provenance that has no OKF field, such as a source marker, goes in Frontmatter.extra and is not added as a new top-level field. capture also refuses to overwrite an existing slug; learn has no such guard. See src/capture.rs:write.
