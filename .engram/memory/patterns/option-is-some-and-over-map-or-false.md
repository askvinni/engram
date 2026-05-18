---
title: "Prefer is_some_and() over map_or(false, ...) to pass clippy -D warnings"
read_when:
  - "writing an Option predicate in Rust code"
  - "debugging a clippy failure in CI on a map_or call"
tripwires:
  - action: "Writing option.map_or(false, |x| condition(x)) in new Rust code"
    warning: "clippy -D warnings will fail CI; use option.is_some_and(|x| condition(x)) instead (available since Rust 1.70)"
last_updated: "2026-05-18"
source_issues: [36]
---

The CI clippy job runs with -D warnings, which treats all lint warnings as errors. Writing Option::map_or(false, |x| f(x)) triggers the clippy::option_map_or_none lint (or clippy::map_or_false depending on the version); the idiomatic replacement is Option::is_some_and(|x| f(x)), available since Rust 1.70. This pattern can appear anywhere in the codebase when filtering on option-typed fields, making it a recurring CI failure if not caught early. See src/claude.rs for the instance fixed in this PR.
