---
title: "engram read stem scan only covers cfg.memory.default_categories — files in unconfigured categories are invisible to stem lookup"
read_when:
  - "debugging why engram read <slug> cannot find a file that exists on disk under .engram/memory/"
  - "adding memory files to a custom category not listed in default_categories and expecting them to be discoverable by name"
tripwires:
  - action: "Creating a memory file in a category absent from cfg.memory.default_categories and relying on engram read <stem> to find it"
    warning: "scan_memory_matches iterates only over default_categories — the file is invisible to stem lookup; use an exact path (e.g. engram read custom-cat/foo.md) or add the category to default_categories in .engram/config.toml"
last_updated: "2026-09-08"
source_issues: [96]
---

scan_memory_matches() in src/main.rs accepts a categories slice from cfg.memory.default_categories and only iterates those directories. A memory file written to a category not present in that config list will never appear as a stem-scan match, even though it sits under .engram/memory/ and is fully valid. The exact-path branch (the first resolution step in cmd_read) has no such restriction and resolves any path under .engram/memory/ regardless of category configuration — so the workaround is always available, but the asymmetry is non-obvious. This affects anyone who adds a new memory category via write_topic_file without also updating default_categories, and any future resolution function built on top of scan_memory_matches. See src/main.rs:scan_memory_matches and src/main.rs:cmd_read.
