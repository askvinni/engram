---
type: tripwire
title: 'engram dispatch must work exactly as typed: auto mode denies command lines that configure an agent or pass a credential'
read_when:
- adding configuration or credentials that engram dispatch (or any command that launches Claude sessions) needs at run time
- writing skill or README instructions that tell an agent how to invoke engram dispatch
- debugging an auto-mode classifier denial ('Create Unsafe Agents', 'Auto-Mode Bypass') when an agent runs engram dispatch
tripwires:
- action: Requiring an env-var prefix, `cd … &&`, or a wrapper (GH_TOKEN=…, ENGRAM_DISPATCH_CLAUDE=…, ./target/debug/engram) on the engram dispatch command line
  warning: The auto-mode classifier judges the command text it sees and denies lines that launch an agent and hand it a credential. Put settings in .engram/config.toml ([dispatch] claude) or git config (engram.githubUser). ENGRAM_DISPATCH_CLAUDE is only for tests.
- action: Implementing or looking for an engram init step that writes Bash(engram dispatch:*) into .claude/settings.json, or a doctor rule check
  warning: 'That part of the #154 spec wasn''t built. A real auto-mode dispatch worked without any permission rule, so the rule is only an optional README tip. engram never writes Claude settings files.'
generated:
  by: engram/0.1.0
  at: 2026-10-27
sources:
- id: '154'
  resource: https://github.com/askvinni/engram/issues/154
---

On 2026-10-22, dispatching from an auto-mode Claude Code session was denied twice. The command spelled out `GH_TOKEN=$(gh auth token …) claude` in an env prefix. kdispatch passed because its launch was hidden inside a script. The fix was to keep the command line plain, not to add permission rules. In auto mode, broad allow rules such as Bash(*engram dispatch *) are suspended, and prefix rules never match a line that starts with `cd` or an assignment anyway. autoMode.allow is read only from user-level or managed settings. So everything dispatch needs must come from config files, and the engram-dispatch skill tells agents to run `engram dispatch <args> --dry-run`, confirm with the user, then run `--yes`, exactly as written. Don't try to hide the launch to get past the classifier. If it still denies a plain invocation, the fallback is a user-written autoMode.allow sentence in ~/.claude/settings.json. Any future engram command that starts agents follows the same rule.
