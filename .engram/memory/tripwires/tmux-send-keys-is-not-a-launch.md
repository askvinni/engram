---
type: tripwire
title: 'Driving a kwt/Ghosthub tmux pane: drop TMUX_TMPDIR, cd explicitly, pass the prompt via a file, and check the pane command afterwards'
read_when:
- sending keystrokes into a tmux pane from engram (dispatch launch or any future relaunch command)
- debugging engram dispatch reporting a launch while no Claude session is running in the pane
- debugging tmux 'no server running' or 'can't find pane' errors from engram while kwt sessions are visible in Ghosthub
tripwires:
- action: Spawning tmux from engram without removing TMUX_TMPDIR and passing -L <socket>
  warning: kwt and Ghosthub run tmux with TMUX_TMPDIR unset, so an inherited value points at a different, empty server and every pane lookup fails. Use Command::env_remove("TMUX_TMPDIR") and the socket name from kwt open --json.
- action: Treating a successful tmux send-keys as proof that claude launched
  warning: 'send-keys succeeds even when claude exits at once (expired auth, a deleted cwd, a trust prompt). Poll #{pane_current_command} until it is no longer a shell, wait the settle delay, and fail with the pane tail if it went back to a shell.'
- action: Inlining a plan or issue body into send-keys
  warning: Bodies contain backticks, $ and quotes that the pane shell will interpolate. Write the body to a file under $XDG_STATE_HOME/engram/dispatch (outside the worktree, so git status stays clean) and send claude "$(cat <file>)" instead.
generated:
  by: engram/0.1.0
  at: 2026-10-27
sources:
- id: '143'
  resource: https://github.com/askvinni/engram/issues/143
---

kdispatch found these constraints by trial and error, and src/dispatch.rs:launch_claude ports them. The pane shell inherits the tmux server's cwd, which can be a deleted directory, and Claude refuses to start there, so the typed command must begin with `cd <worktree> &&`. A pane whose current command is not one of the SHELLS list is already running something: report it as running and do not type into it. Never call `kwt open` without `--start-session`, because the other modes attach the calling terminal. It sometimes fails transiently with 'respawn pane failed: fork failed', and because `--start-session` is idempotent it is retried with 1s and 2s backoff. ENGRAM_DISPATCH_CLAUDE replaces the launched command so tests can use a fake. Any future command that resumes or relaunches a dispatched session must follow the same rules.
