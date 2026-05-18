---
title: "When building @path reference lists, filter to existing files and sort for determinism"
read_when:
  - "building a list of @path references for CLAUDE.md or index.md"
  - "implementing memory enumeration in memory.rs"
  - "regenerating index.md after adding or removing topic files"
tripwires: []
last_updated: "2026-05-18"
source_issues: [8]
---

When enumerating memory files to build a reference list, always (1) filter to files that actually exist on disk — a reference to a deleted file will produce a broken @path import, and (2) sort by path for deterministic output — without sorting, readdir order varies by filesystem and produces noisy diffs on each rebuild. Both conditions are easy to satisfy with a filter_map + sort_by_key after the readdir call. See src/memory.rs:rebuild_index for the pattern.
