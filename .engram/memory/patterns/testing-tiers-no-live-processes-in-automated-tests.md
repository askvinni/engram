---
title: "Tests requiring live gh/git/claude are manual E2E only — never automate them"
read_when:
  - "adding new tests to the engram project"
  - "deciding whether to mock or directly invoke an external process in a test"
tripwires:
  - action: "Writing a cargo test that shells out to gh, git, or claude"
    warning: "These require live credentials and environment that can't run in CI; scope them as manual E2E only and test the pure inner logic in a unit or temp-dir integration test instead"
last_updated: "2026-05-18"
source_issues: [35]
---

The engram test suite has three explicit tiers: (1) unit tests for pure, side-effect-free functions with no I/O, (2) integration tests that write to temp dirs but make no network or process calls, and (3) manual E2E tests that require a live gh, git, or claude binary. Only tiers 1 and 2 run under `cargo test`. The boundary exists because gh, git, and claude each require credentials, a real repo, or a running Claude Code session that cannot be reliably provisioned in CI. When adding a new command that orchestrates these tools, extract the pure logic into a helper and cover that helper with a unit or temp-dir test — leave the orchestration layer as manual-only.
