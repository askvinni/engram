---
type: testing
title: Test tool-availability branches through a pure decision function, because OnceLock caches can't be toggled in tests
read_when:
- writing tests for code that branches on gh_available, kata_available, or another cached external-tool check
- adding a new OnceLock-cached availability probe for an external CLI
tripwires:
- action: Writing a test that asserts behavior when gh (or kata) is missing by calling the real availability check
  warning: gh_available() is cached in a process-wide OnceLock and its value depends on the test machine, so the 'missing gh' branch can't be reached reliably. Move the decision into a pure fn that takes booleans, like config::github_unavailable_reason(gh_ok, configured, remote_ok), and test that function directly.
generated:
  by: engram/0.1.0
  at: 2026-10-19
sources:
- id: '94'
  resource: https://github.com/askvinni/engram/issues/94
---

External-tool probes such as config::gh_available are cached for the whole process in a OnceLock. A test can't change the result, and the result differs between a developer laptop (gh installed) and CI (possibly not). Tests that call resolve_repo end to end can only cover the branches the current machine happens to reach, such as the 'no remote' case in a fresh tempdir git repo. The fix used here is to pass the probe results into a pure function, github_unavailable_reason(gh_ok, configured, remote_ok), so every combination can be tested deterministically, including the rule that a missing gh is reported first. The real filesystem-dependent pieces, has_remote and infer_repo, are tested separately against tempdir repos created with git init. Any new availability-gated logic should follow the same split. See src/config.rs tests.
