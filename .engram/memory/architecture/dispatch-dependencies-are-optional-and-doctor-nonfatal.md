---
type: architecture
title: Dispatch-only tools (kwt, tmux, kata) must never become requirements for other engram commands or fail doctor
read_when:
- adding a preflight or tool-availability check to an engram command
- changing cmd_doctor's checks or its exit status
- wiring an optional feature that depends on extra external binaries
tripwires:
- action: Calling dispatch::preflight() from a command other than dispatch
  warning: init, plan, capture, search and learn must keep working on machines without kwt, tmux or kata. Only engram dispatch may require the full toolchain.
- action: Folding the dispatch availability result into cmd_doctor's all_ok
  warning: The dispatch line is informational and uses its own marker. If it affects all_ok, doctor exits non-zero on every machine that just doesn't dispatch.
generated:
  by: engram/0.1.0
  at: 2026-10-27
sources:
- id: '142'
  resource: https://github.com/askvinni/engram/issues/142
---

engram dispatch needs gh, kata, kwt, tmux and claude, but most users run engram only for memory. The design isolates that toolchain: only dispatch calls preflight, and doctor prints either '✓ dispatch available' or '- dispatch unavailable (missing: …)' (from unavailable_reasons) without touching the existing all_ok, so doctor's exit status still depends only on the core checks. Probes are cached per process with OnceLock, following the existing gh_available()/kata_available() pattern, so a doctor run doesn't spawn five extra version probes more than once. Treat any future optional feature with its own binaries the same way: a separate availability line, and a hard requirement only inside that feature's own command.
