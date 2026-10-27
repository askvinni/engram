---
type: pattern
title: When an engram feature replaces a tool that lives in another repo, merge the removal PR there after the engram stack is merged and installed
read_when:
- retiring or deleting an external tool or script that an engram command now replaces
- opening a PR in a second repo (e.g. agentic-workflows) that depends on an engram PR stack
- replacing a tool's config (git config keys, CLI flags) with an engram [section] in .engram/config.toml
tripwires:
- action: Merging the PR that deletes the old tool before the engram PRs that replace it are merged and installed
  warning: Users are left with no working tool. Open the removal PR in the other repo last, link it from the engram PR, and note in it that it merges after the engram stack.
generated:
  by: engram/0.1.0
  at: 2026-10-27
sources:
- id: '146'
  resource: https://github.com/askvinni/engram/issues/146
---

kdispatch lived in askvinni/agentic-workflows while engram dispatch was built across a stack of engram PRs (5.1-5.4), so the retirement was split: engram's docs PR (#146) only documented the replacement and removed references, and the deletion went into agentic-workflows#4, which merges after the whole stack. Before removing anything, check that each mode of the old tool is covered and list the features that are dropped on purpose. The old tool's config is not read by engram: kdispatch.* git config is ignored and its templates must be moved into [dispatch]. So the migration path, including the field mapping, has to be in the docs and is not handled automatically. Use the same order for any future tool engram absorbs: ship and document the engram side first, then remove the old tool in its own repo, and clean up any installed links it leaves behind.
