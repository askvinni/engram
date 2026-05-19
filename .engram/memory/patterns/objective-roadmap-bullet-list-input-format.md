---
title: "Supply bullet-list roadmap to `engram objective new` — CLI generates the table and hidden JSON, not the caller"
read_when:
  - "creating an engram-objective GitHub issue from code or a skill"
  - "calling parse_nodes_from_roadmap_input in src/objective.rs"
  - "debugging why --all-unblocked does not dispatch nodes on a newly created objective"
tripwires:
  - action: "Hand-writing the Markdown table or the <!-- engram:nodes [...] --> JSON comment in an objective issue body"
    warning: "engram objective new generates both from the bullet-list roadmap section — writing them manually produces either a missing JSON state comment (breaking --all-unblocked and auto-close) or a malformed structure that silently fails to parse"
last_updated: "2026-05-19"
source_issues: [61]
---

The `engram objective new` command accepts a `## Roadmap` section written as a bullet list (`- 1.1: Description`, `- 1.2: Description (depends: 1.1)`) and passes it to `parse_nodes_from_roadmap_input` in `src/objective.rs`, which generates both the rendered Markdown table and the hidden `<!-- engram:nodes [...] -->` JSON state comment. Any caller — skill, test, or Rust function — that instead hand-writes the table or JSON directly will produce an issue body that either lacks the machine-readable JSON (making the objective invisible to `--all-unblocked` and `maybe_close_objective`) or contains structure that silently fails to parse at dispatch time. Always supply the bullet-list format and let the CLI do the transformation. See `src/objective.rs:parse_nodes_from_roadmap_input` and the node bullet format in `.claude/skills/engram-objective/SKILL.md`.
