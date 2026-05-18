---
title: "engram init migration produces placeholder read_when values that break self-routing until manually filled"
read_when:
  - "running engram init on a repo that has existing flat .engram/memory/*.md files"
  - "investigating why the memory index shows '(migrated — add read_when conditions)' entries"
  - "implementing or testing the flat-file migration path in src/memory.rs"
tripwires:
  - action: "Treating migrated topic files as fully functional after engram init completes"
    warning: "Migrated files get '(migrated — add read_when conditions)' as their read_when value — the routing table will surface them but no agent task condition will match, so the files are never loaded until read_when is filled in manually"
last_updated: "2026-05-18"
source_issues: [29]
---

When `engram init` migrates existing flat category files (e.g., `.engram/memory/architecture.md`) to the new per-topic layout, it creates individual `.md` files with a placeholder `read_when` value of `(migrated — add read_when conditions)`. This placeholder is syntactically valid YAML but semantically useless — no agent's current-task description will match it, so the migrated learning is unreachable via self-routing even though it appears in `index.md`. After running `engram init` on a legacy repo, each migrated file must be manually edited to add real read_when conditions before the memory becomes effective. Tests that verify memory routing after migration should assert on the presence of real conditions, not just the absence of empty lists. See src/memory.rs for the migration function.
