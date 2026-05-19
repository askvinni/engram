---
title: "Top-level engram learn/land/list/status no longer exist — all are now under engram plan"
read_when:
  - "generating or scripting any engram CLI command involving learn, land, list, or status"
  - "updating documentation, skills, or memory that previously referenced engram learn or engram land"
tripwires:
  - action: "Generating the command `engram learn <N>` or `engram land <N>` or `engram list` or `engram status`"
    warning: "These top-level subcommands were removed — the correct forms are `engram plan learn <N>`, `engram plan land <N>`, `engram plan list`, `engram plan status`; the old forms return 'unrecognized subcommand' with no migration hint"
  - action: "Creating a new plan issue with `engram plan <title>` (positional argument form)"
    warning: "This form was replaced — the correct command is `engram plan new "<title>"`"
last_updated: "2026-05-19"
source_issues: [69]
---

As of issue #69, all five plan-related top-level commands were regrouped under the `engram plan` subcommand with no backwards-compatibility aliases: `engram plan new` (was `engram plan <title>`), `engram plan learn <N>` (was `engram learn <N>`), `engram plan land <N>` (was `engram land <N>`), `engram plan list` (was `engram list`), and `engram plan status` (was `engram status`). Invoking the old forms produces 'unrecognized subcommand' with no hint about the new location — the breaking change was intentional and accepted with no migration shim. Any script, CI job, skill, or memory file referencing the old command forms must be updated. See src/cli.rs:PlanCommands for the authoritative subcommand list.
