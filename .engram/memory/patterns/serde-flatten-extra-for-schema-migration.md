---
type: pattern
title: 'Use #[serde(flatten)] extra: Map<...> to preserve unknown keys when migrating a hand-rolled format to a typed struct'
read_when:
- replacing string-based or format!-built frontmatter/config parsing with a typed serde struct
- writing a migration command that must round-trip files containing fields the new struct doesn't know about
- adding a new schema version that needs to coexist with files written by older code or third-party tools
tripwires: []
generated:
  by: engram/0.1.0
  at: 2026-10-12
sources:
- id: '131'
  resource: https://github.com/askvinni/engram/issues/131
---

When converting a format that was previously read/written via string matching (e.g. format!-built YAML frontmatter) into a typed struct, add a #[serde(flatten)] extra field of a generic map type alongside the named fields. Without it, any key not explicitly modeled — whether from an older schema version, a hand-edited file, or a different tool that writes into the same file — is silently dropped on the first rewrite. This matters for any migration command (not just one-off scripts) because the whole point of a migrate command is to be safely re-runnable and non-destructive; flattening unknown keys into a catch-all is what makes that guarantee verifiable rather than assumed.
