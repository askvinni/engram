---
title: "Globbing a prompt-hooks directory loads README.md as a rule…"
read_when:
  - "(migrated — add read_when conditions)"
tripwires: []
last_updated: "2026-05-18"
source_issues: [26]
---

Globbing a prompt-hooks directory loads README.md as a rule file unless explicitly filtered — always skip files by documentation naming convention (README.md, etc.) when loading prompt customization files.
