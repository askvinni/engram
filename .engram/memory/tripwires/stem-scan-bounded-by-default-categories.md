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

scan_memory_matches() only searches directories listed in cfg.memory.default_categories. A file in an unlisted category is invisible to stem lookup but reachable via exact path (the first resolution step in cmd_read has no category restriction). See src/main.rs:scan_memory_matches.
