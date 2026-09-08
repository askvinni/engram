---
title: "Index-reading functions must check DB existence and rebuild on demand rather than error"
read_when:
  - "adding a new function to index.rs that reads from the FTS5 index"
  - "debugging why an index read fails on a fresh clone or after deleting .engram/index.db"
tripwires:
  - action: "Returning an error or propagating a 'file not found' condition when the index DB is absent in an index-reading function"
    warning: "The established contract is to call rebuild_index() when DB_RELATIVE does not exist before opening the connection — erroring on a missing DB breaks cold-start and clean-slate workflows"
last_updated: "2026-09-08"
source_issues: [92]
---

The search() function in src/index.rs establishes the design contract for all index-reading functions: check whether the DB file at DB_RELATIVE exists and, if not, call rebuild_index() before opening the connection. This silent on-demand rebuild means cold-start users (fresh clones, deleted index files) get a working result on first run rather than an error asking them to run a separate init step. Any future function that reads from the index must apply the same existence check rather than assuming the DB is present. check_index_health() exists only for reporting purposes and is not a substitute. See src/index.rs:search.
