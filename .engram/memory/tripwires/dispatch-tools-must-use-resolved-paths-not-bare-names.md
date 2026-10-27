---
type: tripwire
title: 'Call dispatch tools by the path preflight resolved, never by bare name: kwt is often only a shell alias'
read_when:
- implementing an engram dispatch step that shells out to kwt, tmux, kata, gh or claude
- debugging 'No such file or directory' from Command::new("kwt") when kwt works in the user's terminal
- adding a new external tool to the dispatch preflight Toolchain
tripwires:
- action: Calling Command::new("kwt") (or any dispatch tool by bare name) inside a dispatch step
  warning: kwt is often only a zsh alias to /Applications/Ghosthub.app/Contents/Helpers/kwt, so a bare-name spawn fails even though the user can type kwt. Use the path held in dispatch::preflight's Toolchain instead.
- action: Probing tmux availability with `tmux --version`
  warning: tmux has no --version flag. Use `tmux -V`, which also avoids starting a server or depending on TMUX_TMPDIR.
generated:
  by: engram/0.1.0
  at: 2026-10-27
sources:
- id: '142'
  resource: https://github.com/askvinni/engram/issues/142
---

Rust's Command searches PATH only. It never sees shell aliases, and many users reach kwt only through an alias to the copy bundled with Ghosthub. dispatch::preflight resolves each tool once: PATH first, then the Ghosthub fallback for kwt if that file exists and is executable. It returns a Toolchain of absolute paths, so every later dispatch step (worktree creation, tmux launch, claim) must spawn through those paths. Do not resolve tools through a login shell; that was rejected as slow and fragile. Preflight checks every tool and reports all missing ones in a single error. It checks the kata binary separately from the repo's .kata.toml binding, so 'kata not installed' and 'repo not bound (run kata init)' show up as different problems. See src/dispatch.rs:resolve_tool and tool_problems.
