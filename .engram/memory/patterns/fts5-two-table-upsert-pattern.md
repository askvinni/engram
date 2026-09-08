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

FTS5 virtual tables only support row access by internal rowid. Without the file_index companion table (path TEXT PRIMARY KEY, fts_rowid INTEGER), targeted single-file updates require a full table scan. See src/index.rs:upsert_row.
