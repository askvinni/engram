---
title: "rusqlite must declare features = ["bundled"] — system SQLite often omits FTS5"
read_when:
  - "modifying the rusqlite dependency version or features in Cargo.toml"
  - "debugging a 'no such module: fts5' error at runtime after a dependency update"
  - "adding a new crate that links SQLite to the engram binary"
tripwires:
  - action: "Declaring rusqlite in Cargo.toml without features = ["bundled"]"
    warning: "The build succeeds but links against system libsqlite3, which many Linux distros compile without FTS5 — the memory_fts virtual table creation fails at runtime with 'no such module: fts5'; always use bundled to guarantee FTS5 availability"
last_updated: "2026-09-08"
source_issues: [84]
---

FTS5 is a compile-time option in SQLite, not part of the default build on all platforms. System-packaged SQLite on many Linux distributions is built without it, so linking against the host's libsqlite3 produces a binary that compiles cleanly but crashes at first run when attempting to create the memory_fts virtual table. The rusqlite bundled feature compiles a pinned SQLite version (with FTS5 enabled) directly into the engram binary, eliminating the host library dependency entirely. This is not a nice-to-have — without it the entire index module is non-functional on a wide class of deployment targets. See Cargo.toml.
