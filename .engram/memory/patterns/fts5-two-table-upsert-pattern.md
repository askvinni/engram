---
title: "FTS5 upsert requires a companion path→rowid table for O(1) targeted updates"
read_when:
  - "implementing upsert_file() or any incremental index update in index.rs"
  - "adding a delete or update operation against the memory_fts FTS5 virtual table"
  - "designing a new FTS5-backed index for any engram data type"
tripwires:
  - action: "Deleting or replacing a specific FTS5 row by file path without the file_index companion table"
    warning: "FTS5 only supports efficient row access by internal rowid — without file_index's path→fts_rowid mapping you must full-scan the FTS table to find the row; always look up the rowid from file_index before mutating memory_fts"
last_updated: "2026-09-08"
source_issues: [84]
---

SQLite FTS5 virtual tables only support efficient row access by their internal rowid — there is no indexed lookup by an arbitrary column value like a file path. Without a companion table mapping path to fts_rowid, upsert_file() would have no way to delete a specific file's stale FTS row without a full table scan, making incremental updates O(n) in index size. The file_index table (path TEXT PRIMARY KEY, fts_rowid INTEGER NOT NULL) solves this: upsert_file reads the rowid, deletes the old FTS row by rowid, inserts the new content, and stores the new rowid. rebuild_index() bypasses file_index by clearing all rows and reinserting from disk, but any targeted single-file update path depends on this companion table. See src/index.rs:upsert_file and migrations/001_initial_schema.sql.
