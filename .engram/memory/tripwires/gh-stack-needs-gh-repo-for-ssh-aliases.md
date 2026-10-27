---
type: tripwire
title: Run gh stack (and other gh extensions) with GH_REPO set, because gh-stack can't resolve SSH host aliases
read_when:
- shelling out to `gh stack link` or another gh extension from engram
- debugging gh stack link failing or linking the wrong repo when the origin remote uses an SSH host alias
- adding a new wrapper in src/github.rs that calls a gh extension rather than a core gh command
tripwires:
- action: Calling `gh stack link` with no GH_REPO, relying on gh to infer the repo from the git remote
  warning: gh-stack can't map SSH host aliases (e.g. git@github-work:owner/repo), so repo detection fails even though core gh commands work. Spawn it through github::gh_stack, which sets GH_REPO to the resolved owner/repo.
- action: Treating a failed or missing `gh stack link` as a dispatch failure
  warning: Linking is cosmetic. The PRs are already chained by their base branches, so warn (and name `gh stack link`) instead of failing the command.
generated:
  by: engram/0.1.0
  at: 2026-10-27
sources:
- id: '145'
  resource: https://github.com/askvinni/engram/issues/145
---

Core gh commands resolve the repository through gh's own remote handling, which understands SSH host aliases from ~/.ssh/config. gh extensions such as gh-stack do their own remote parsing and don't. On machines with an aliased origin, which is common when people use several GitHub accounts, they fail or target the wrong repo. Stacked dispatch hit this, so src/github.rs:gh_stack runs the extension with GH_REPO set explicitly. Any future gh extension call should be routed the same way and not assume the remote is detected. A link failure only produces a warning, because each stacked PR's --base already encodes the chain; gh stack link only adds the stack UI on top.
