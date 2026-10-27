---
type: tripwire
title: Dispatch templates shipped without {kata_ref} or {{ }} escapes and default to {number}-{slug}, unlike the 5.1 issue and kdispatch
read_when:
- rendering a dispatch branch name or PR title from a [dispatch] template
- documenting the [dispatch] config section or migrating users from kdispatch
- 'implementing a dispatch node (5.2+) from the objective #141 issue text'
tripwires:
- action: Using {kata_ref}, {short_id}, or {{/}} escapes in a [dispatch] template, or calling dispatch::render
  warning: The merged code is dispatch::fill_template over PlanFields. It supports only {number}, {title}, {slug} and {meta[key]}, and any other placeholder fails as unknown. The issue's render/{kata_ref}/escape spec was not implemented.
- action: Assuming a user's kdispatch.branch git config carries over to engram dispatch
  warning: engram ignores kdispatch.* git config and defaults to {number}-{slug} (the GitHub plan number), not kdispatch's {short_id}-{slug}. Users must copy their templates into [dispatch], and the docs must say so.
generated:
  by: engram/0.1.0
  at: 2026-10-27
sources:
- id: '142'
  resource: https://github.com/askvinni/engram/issues/142
---

Issue #142 specified a render() function with {kata_ref} and literal-brace escapes, but the merged PR shipped a narrower fill_template, so agents working from the issue text will reach for an API that doesn't exist. Missing meta keys are all collected and reported together, each with its `kata meta set <ref> <key> <value>` fix command. slugify also differs from the kdispatch port described in the issue: it drops a leading '[5.1]'-style node prefix and cuts at a word boundary within 40 characters. {title} stays unsanitized because it only goes into PR titles, so check rendered branch names as valid refs (git check-ref-format --branch) before anything is mutated. Empty slugs or meta values containing spaces or '..' otherwise fail only at push time. Unknown [dispatch] keys, such as the kdispatch-style prTitle, warn once and are ignored rather than rejected. See src/dispatch.rs:fill_template and src/config.rs:DispatchConfig.
