---
title: "Compose higher-level workflow commands by calling existing …"
read_when:
  - "(migrated — add read_when conditions)"
tripwires: []
last_updated: "2026-05-18"
source_issues: [11]
---

Compose higher-level workflow commands by calling existing command functions internally (e.g., `land` calls `learn::run()`) rather than duplicating logic across subcommands.
