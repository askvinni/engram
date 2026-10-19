---
type: tripwire
title: engram search rebuilds the FTS index only when it is missing, so new memory files must be upserted
read_when:
- adding a new code path that writes, renames, or deletes a file under .engram/memory/
- debugging why engram search does not return a memory file that exists on disk
tripwires:
- action: Writing a new memory file and only calling rebuild_index() and write_claude_md_section()
  warning: Those calls update index.md and CLAUDE.md, not the SQLite FTS index. If index::check_index_health() is true, also call index::upsert_file(), or engram search will never find the new file.
generated:
  by: engram/0.1.0
  at: 2026-10-19
sources:
- id: '86'
  resource: https://github.com/askvinni/engram/issues/86
---

engram has two separate indexes. The markdown routing table (.engram/memory/index.md) is rebuilt by memory::rebuild_index. The SQLite FTS index, used by engram search, is rebuilt only when it is missing or unhealthy. So any write path that skips the FTS index leaves an existing index stale, and search silently misses the new file. Nothing errors, and the file still shows up in index.md and engram read. capture handles this: after writing, it checks index::check_index_health and, if the index exists, calls index::upsert_file with the repo-relative path, category, and slug. Any other command that changes memory files must do the same (learn, compact, memory migrate, and any future import or edit command). See src/capture.rs:write.
