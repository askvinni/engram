---
title: "Plan body validation accepts **What** as an alias for **Acceptance criteria**"
read_when:
  - "modifying or extending missing_plan_sections() in src/main.rs"
  - "renaming a required section in the engram plan body format"
tripwires:
  - action: "Removing the '**What**' alias check from missing_plan_sections()"
    warning: "All plan issues opened before issue #49 use '**What**' as the section name — removing the alias produces false warnings for every pre-existing plan body re-evaluated by the validation"
last_updated: "2026-05-18"
source_issues: [49]
---

missing_plan_sections() in src/main.rs accepts either '**Acceptance criteria**' or '**What**' as satisfying the acceptance-criteria requirement. Plan issues opened before issue #49 used '**What**' as the section header. This alias is the only thing preventing every pre-existing plan body from generating a spurious missing-section warning. The alias is invisible from SKILL.md or the issue template — it exists only in the validation function. Any future section rename must preserve a backward-compatible alias for the same reason, and any refactor that tightens the match logic must confirm the alias is still present. See src/main.rs:missing_plan_sections.
