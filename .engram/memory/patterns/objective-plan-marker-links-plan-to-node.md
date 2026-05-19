---
title: "Plans linked to objective nodes carry an `Objective: #N (node id)` prefix that cmd_land reads to auto-update node status"
read_when:
  - "creating a plan issue body that should auto-update an objective node when it lands"
  - "implementing or changing maybe_mark_node_done() in objective.rs"
  - "adding a new code path that creates engram-plan issues on behalf of an objective node"
tripwires:
  - action: "Creating an engram-plan issue body for an objective node without the `Objective: #N (node id)` prefix"
    warning: "cmd_land calls maybe_mark_node_done() which parses this prefix to locate and update the objective — omitting it silently skips the node update with only a stderr warning, so the objective is never marked done"
last_updated: "2026-05-19"
source_issues: [54]
---

`objective::plan` prepends `Objective: #<objective_number> (node <node_id>)` to the generated plan issue body before creating it on GitHub. `cmd_land` calls `objective::maybe_mark_node_done`, which parses this prefix from the landed plan's body and uses the extracted numbers to fetch the objective issue, update the matching node to `done`, and write the body back. The coupling is purely textual — there is no foreign-key relationship in the GitHub API — so any future code path that creates plan issues on behalf of an objective must include this exact prefix in the body. The failure mode in cmd_land is non-fatal by design (stderr warning only), meaning a missing or malformed marker silently skips the objective update rather than blocking the land workflow. See src/objective.rs:maybe_mark_node_done and src/main.rs:cmd_land.
