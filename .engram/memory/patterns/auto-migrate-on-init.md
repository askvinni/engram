---
title: "Trigger storage format migrations automatically in the init command"
read_when:
  - "evolving the on-disk format of a CLI tool's stored state"
  - "adding a new directory layout that supersedes a flat file structure"
  - "deciding where to place a one-time migration for existing users"
tripwires: []
last_updated: "2026-05-18"
source_issues: [29]
---

When a CLI tool's storage format changes (e.g. flat files → subdirectory tree), triggering the migration inside the init command makes the upgrade transparent: users who run init on an existing repo get migrated automatically, and new users never encounter the old layout. This is preferable to a dedicated migration subcommand because it removes the burden of communicating a required upgrade step. The migration function should be idempotent — check whether old-format files exist before moving them — so re-running init on an already-migrated repo is a no-op. See src/main.rs and src/memory.rs:migrate_flat_files for the implementation pattern.
