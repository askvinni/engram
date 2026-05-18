# Engram Memory Index

Agents: read this index to find relevant learned docs. Load individual files only when their "Read when" condition matches your current task.

| File | Title | Read when |
|------|-------|-----------|
| @.engram/memory/patterns/check-resource-state-before-mutating-it-e-g-check-issue-stat.md | Check resource state before mutating it — external systems may have already acted | calling a mutation on a GitHub resource (close issue, delete branch, merge PR); implementing a workflow step that may already have been done by a prior step; adding a new operation to the land command |
| @.engram/memory/patterns/option-is-some-and-over-map-or-false.md | Prefer is_some_and() over map_or(false, ...) to pass clippy -D warnings | writing an Option predicate in Rust code; debugging a clippy failure in CI on a map_or call |
| @.engram/memory/patterns/rust-ci-toolchain-components.md | Use dtolnay/rust-toolchain with explicit components for Rust CI | adding or modifying a GitHub Actions workflow for a Rust project; adding a clippy or rustfmt job to CI |
| @.engram/memory/patterns/strip-markdown-code-fences-from-llm-output-before-json-parsi.md | Strip markdown code fences from LLM output before JSON parsing | parsing structured output from claude -p; adding a new synthesis function in claude.rs that expects JSON back; debugging JSON parse errors from claude output |
| @.engram/memory/tripwires/globbing-a-prompt-hooks-directory-loads-readme-md-as-a-rule-.md | Globbing a prompt-hooks directory loads README.md as a rule file unless excluded | loading files from a directory that may contain documentation files; implementing or changing load_prompt_hooks(); adding a new directory where users place configuration files alongside a README |
| @.engram/memory/tripwires/inlining-memory-content-into-claude-md-is-unsafe-if-any-lear.md | Inlining memory content into CLAUDE.md corrupts section boundaries if content contains the end marker | writing memory content into CLAUDE.md; implementing or changing write_claude_md_section(); changing the engram section delimiter strings |
| @.engram/memory/tripwires/invoking-claude-p-from-within-a-repo-directory-causes-claude.md | Invoking claude -p from inside a repo directory loads CLAUDE.md as agent context | calling claude -p programmatically from engram; implementing or changing synthesize_learnings() or any function that shells out to Claude; debugging unexpected Claude behavior where it acts as an agent instead of returning JSON |
