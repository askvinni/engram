---
title: "Use dtolnay/rust-toolchain with explicit components for Rust CI"
read_when:
  - "adding or modifying a GitHub Actions workflow for a Rust project"
  - "adding a clippy or rustfmt job to CI"
tripwires:
  - action: "Using actions-rs/toolchain in a GitHub Actions workflow"
    warning: "actions-rs/toolchain is unmaintained; use dtolnay/rust-toolchain@stable instead to avoid deprecation failures"
  - action: "Adding a clippy or fmt CI job without a components declaration"
    warning: "The default stable toolchain does not include clippy or rustfmt; omitting 'components: clippy' or 'components: rustfmt' causes the job to fail with a tool-not-found error"
last_updated: "2026-05-18"
source_issues: [36]
---

When setting up Rust CI in GitHub Actions, always use dtolnay/rust-toolchain@stable rather than actions-rs/toolchain — the latter is unmaintained and will fail or emit deprecation errors. The default stable toolchain install does not bundle clippy or rustfmt; each job that uses these tools must declare them explicitly via the 'components:' key in the toolchain step. The fmt job does not need Swatinem/rust-cache because cargo fmt --check never compiles code — cache there wastes time without benefit. See .github/workflows/ci.yml for the reference pattern.
