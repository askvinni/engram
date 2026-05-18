---
title: "Strip markdown code fences from LLM output before JSON pars…"
read_when:
  - "(migrated — add read_when conditions)"
tripwires: []
last_updated: "2026-05-18"
source_issues: [26]
---

Strip markdown code fences from LLM output before JSON parsing — use a `strip_code_fence()` step that checks for a ` ``` ` first line and extracts only the inner lines, because Claude may wrap responses in ` ```json ... ``` ` blocks even when instructed not to.
