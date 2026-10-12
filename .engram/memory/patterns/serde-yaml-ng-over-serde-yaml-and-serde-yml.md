---
type: pattern
title: Prefer serde_yaml_ng for new Rust YAML work — serde_yaml is archived, serde_yml has known soundness issues
read_when:
- adding a YAML dependency to a Rust project
- seeing a deprecation or archival notice on serde_yaml and looking for a replacement
tripwires: []
generated:
  by: engram/0.1.0
  at: 2026-10-12
sources:
- id: '131'
  resource: https://github.com/askvinni/engram/issues/131
---

serde_yaml, long the default choice for YAML in Rust, is archived and unmaintained. The most visible drop-in successor, serde_yml, has reported soundness issues and should be avoided. serde_yaml_ng is the maintained fork that preserves the serde_yaml API without those problems, and is the safer default for any new or migrated YAML (de)serialization in a Rust codebase.
