---
title: "Embed SQL migrations with include_dir and track applied count via PRAGMA user_version"
read_when:
  - "adding a new schema change to the engram SQLite index"
  - "implementing or changing run_migrations() in index.rs"
  - "understanding why schema changes require a new numbered file rather than editing an existing one"
tripwires:
  - action: "Editing an existing file in migrations/ to add or change schema"
    warning: "run_migrations() skips any migration file whose sort-position is at or below the stored PRAGMA user_version — modifications to already-applied files are silently ignored on existing databases; always create a new numerically-prefixed file for every schema change"
last_updated: "2026-09-08"
source_issues: [84]
---

Migration SQL files live under migrations/ and are embedded into the engram binary at compile time via include_dir!("$CARGO_MANIFEST_DIR/migrations"). run_migrations() reads PRAGMA user_version to determine how many files have already been applied, sorts embedded files by filename, and applies only those beyond the current count, incrementing user_version after each. This approach needs no runtime file-system access, works identically in test environments and deployed binaries, and requires no separate migration-tracking table. The inviolable rule is: every schema change gets a new numerically-prefixed file; editing an existing file is a silent no-op on any database that already ran it. See src/index.rs:run_migrations and migrations/001_initial_schema.sql.
