---
type: tripwire
title: Claude Code resolves @-prefixed paths inside markdown table cells, relative to the importing file
read_when:
- designing an index or routing file that Claude Code will load via an @ reference
- debugging why a file referenced from an index unexpectedly loads its full content into every session
- writing a table or list that contains file paths meant to be inert text, not live references
tripwires:
- action: Putting an @-prefixed path inside a markdown table cell or list item in a file that is itself @-imported
  warning: Claude Code follows @paths found anywhere in an imported file's content, including table cells, and resolves them relative to that file's own location — not the root CLAUDE.md. A path that fails to resolve today (e.g. because it's written relative to repo root instead of the file's directory) can start eager-loading everything the moment someone 'corrects' it to a working relative path. Use plain (non-@) links for anything meant to be read selectively, and verify with a real claude -p canary, not just inspection of the markdown.
generated:
  by: engram/0.1.0
  at: 2026-10-12
sources:
- id: '131'
  resource: https://github.com/askvinni/engram/issues/131
---

A manual claude -p canary test (tools disabled) confirmed that Claude Code doesn't limit @-import following to top-level CLAUDE.md lines — it recursively follows @-prefixed strings found inside an already-imported file's body, including inside markdown table cells, and resolves them relative to that file rather than the repo root. This means a routing/index file designed for selective, read_when-gated loading must avoid @ syntax entirely in its entries; only the single top-level @path that pulls in the index itself should use @. The risk is invisible from reading the code — the existing broken paths merely happened not to resolve — and a well-intentioned fix to make them resolve correctly would silently turn selective routing into eager-loading everything.
