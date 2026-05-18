<!-- engram:start -->
## Engram Memory

Learnings from past development work:

### architecture
# architecture

- CLAUDE.md is updated in-place using <!-- engram:start --> / <!-- engram:end --> markers, so engram can surgically rewrite only its section without touching user content. _(from #2)
- Learnings live in .engram/memory/<category>.md (patterns, tripwires, architecture, testing), keeping categories as separate files rather than one flat list. _(from #2)


### patterns
# patterns

- The core workflow is linear: plan (issue) → implement → merge PR with 'closes #N' → learn; document this sequence explicitly so users know learn must come last. _(from #2)
- README should list prerequisites with installation links (gh, Claude Code) before commands, since the tool is useless without authenticated CLI dependencies. _(from #2)



<!-- engram:end -->