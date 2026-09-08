---
title: "Mark top-level clap flags `global = true` so they propagate through all subcommands"
read_when:
  - "adding a new top-level flag to the engram Cli struct that must be accessible inside subcommand handlers"
  - "debugging why a top-level flag parses on bare invocations but not on subcommand invocations like `engram --flag subcmd arg`"
tripwires:
  - action: "Adding a top-level `#[arg(long)]` flag to `Cli` without `global = true`"
    warning: "clap will accept the flag only when it appears after the binary name with no subcommand — passing it before a subcommand (e.g. `engram --agent plan list`) will either fail to parse or silently leave the field at its default in the subcommand handler; always set `global = true` for flags that must be visible to every subcommand"
last_updated: "2026-09-08"
source_issues: [88]
---

Without `global = true`, clap treats a flag as belonging only to the root command's argument set — it silently parses as the default value in any subcommand context. See the existing `--agent` flag in src/cli.rs for the reference pattern.
