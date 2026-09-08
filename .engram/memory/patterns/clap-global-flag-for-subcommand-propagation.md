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

clap's default behaviour does not propagate a flag defined on the root `Cli` struct into subcommand parse contexts. Without `global = true`, a top-level `--agent` flag succeeds for bare invocations but silently parses as the default (false) — or outright rejects the command — when a subcommand follows, because clap treats the flag as belonging only to the root command's argument set. Setting `global = true` on the `#[arg]` attribute tells clap to inject the flag into every subcommand's parse context, making it available wherever `Cli` is accessed. This applies to any future cross-cutting flag added to `Cli` that command handlers downstream need to read, such as `--json`, `--quiet`, or `--dry-run`. See src/cli.rs.
