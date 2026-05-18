---
title: "gh CLI --json output uses camelCase field names, not snake_case"
read_when:
  - "adding a new struct to deserialize gh --json output in github.rs"
  - "adding a new gh issue or gh pr JSON query and mapping fields to a Rust struct"
tripwires:
  - action: "Defining a Rust struct to deserialize gh --json output without serde rename attributes"
    warning: "The gh CLI returns camelCase (e.g. createdAt, updatedAt, mergedAt) — serde will silently fail to populate any field whose name differs from the JSON key, so always add #[serde(rename = "camelCaseName")] for multi-word fields"
last_updated: "2026-05-18"
source_issues: [10]
---

The gh CLI serializes issue and PR JSON with camelCase field names (createdAt, updatedAt, mergedAt, etc.) regardless of Rust naming convention. Without explicit #[serde(rename)] annotations, serde_json will silently produce default/None values for mismatched fields — there is no parse error to alert you. This applies to every struct that deserialises gh --json output; see the PlanIssue struct in src/github.rs for the canonical example with created_at.
