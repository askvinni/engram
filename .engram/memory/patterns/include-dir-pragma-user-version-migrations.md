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

Every schema change must be a new numerically-prefixed file under migrations/. Editing an existing migration is a silent no-op on any database that already ran it — run_migrations() applies only files beyond the stored PRAGMA user_version. See src/index.rs:run_migrations.
