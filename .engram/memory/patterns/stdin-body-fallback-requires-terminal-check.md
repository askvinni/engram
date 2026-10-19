---
type: pattern
title: Check IsTerminal before falling back to reading the body from stdin
read_when:
- adding an engram subcommand that accepts content via a flag or from piped stdin
- debugging an engram command that hangs with no output when run without arguments
tripwires:
- action: Calling read_to_string on stdin as the fallback when a --body-style flag is absent
  warning: If stdin is an interactive terminal, the command blocks forever waiting for EOF. Check std::io::stdin().is_terminal() first and bail with a message that says to pass the flag or pipe the input.
generated:
  by: engram/0.1.0
  at: 2026-10-19
sources:
- id: '86'
  resource: https://github.com/askvinni/engram/issues/86
---

Commands that agents call, such as engram capture, take content either from a flag or from piped stdin. If the command reads stdin without checking it, then running it from a shell (or from an agent harness with an attached TTY) and forgetting the flag looks like a hang, not an error. That is especially bad for agents, which may wait until a timeout. The fix is to check std::io::IsTerminal on stdin and fail right away with a usage hint, reading stdin only when it is piped. Any future engram command that takes free-form input this way should reuse this helper or follow the same pattern. See src/capture.rs:resolve_body.
