---
type: tripwire
title: rusqlite must declare features = ["bundled"] — system SQLite often omits FTS5
read_when:
- modifying the rusqlite dependency version or features in Cargo.toml
- 'debugging a ''no such module: fts5'' error at runtime after a dependency update'
- adding a new crate that links SQLite to the engram binary
tripwires:
- action: Declaring rusqlite in Cargo.toml without features = ["bundled"]
  warning: 'The build succeeds but links against system libsqlite3, which many Linux distros compile without FTS5 — the memory_fts virtual table creation fails at runtime with ''no such module: fts5''; always use bundled to guarantee FTS5 availability'
generated:
  by: engram/0.1.0
  at: 2026-09-08
sources:
- id: '84'
  resource: https://github.com/askvinni/engram/issues/84
---

FTS5 is a compile-time SQLite option omitted from many Linux distros' system packages. The bundled feature compiles a pinned SQLite (FTS5 enabled) directly into the binary. Without it the build succeeds but virtual table creation fails at first run. See Cargo.toml.
