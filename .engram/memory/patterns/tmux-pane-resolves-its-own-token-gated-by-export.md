---
type: pattern
title: Have the tmux pane resolve GH_TOKEN itself with `GH_TOKEN="$(gh auth token --user '<u>')" && export GH_TOKEN && claude …`
read_when:
- building the shell command typed into a dispatch or relaunch tmux pane
- passing any credential or environment variable to a Claude session started in a kwt/Ghosthub pane
- printing a launch command in dry-run output or logs
tripwires:
- action: Setting GH_TOKEN on engram's own process and expecting the launched pane to inherit it, or typing a literal token into send-keys
  warning: tmux panes inherit the tmux server's environment, not engram's. A literal token ends up in keystrokes and scrollback. Type a command that resolves the token in the pane instead.
- action: Writing the pane command as `GH_TOKEN="$(…)" claude …` (an assignment prefix on the claude command)
  warning: A failed lookup would still start Claude, with an empty token or as the wrong account. Gate it with `&& export GH_TOKEN &&` so a failure leaves the pane at the shell, where launch detection reports it as failed.
generated:
  by: engram/0.1.0
  at: 2026-10-27
sources:
- id: '154'
  resource: https://github.com/askvinni/engram/issues/154
---

dispatch::launch_command adds the token step only when engram.githubUser is set. Without it, the command is unchanged. The `&&` chain matters: the standalone assignment returns the command substitution's exit status, so a gh auth failure stops the chain before claude runs. The existing check that the pane went back to a shell then reports a failed launch. This requires a POSIX shell in the pane, so fish is not supported, and the README says so. The dry-run launch line passes through redact_secrets, which masks credential assignments in case [dispatch] claude itself contains one. Never write a token into keystrokes, prompt files, logs or error messages. Any future relaunch or resume command should build its pane command the same way. See src/dispatch.rs:launch_command.
