//! The `[dispatch]` branch and PR-title templates.

use anyhow::Result;
use std::collections::BTreeMap;

use crate::integrations::kata;

pub(super) const SLUG_MAX: usize = 40;

/// What `[dispatch]` templates can say about a plan issue.
#[derive(Debug, Default)]
pub(super) struct PlanFields {
    pub(super) number: u64,
    /// The plan's kata mirror, used only in the hint for missing metadata.
    pub(super) kata_ref: Option<String>,
    pub(super) title: String,
    pub(super) meta: BTreeMap<String, String>,
}

/// `title` without a leading node-id bracket such as `[5.1] `; other bracket
/// groups (`[ENG-7] ...`) stay.
pub(super) fn strip_node_prefix(title: &str) -> &str {
    let Some((inner, rest)) = title
        .trim_start()
        .strip_prefix('[')
        .and_then(|r| r.split_once(']'))
    else {
        return title;
    };
    let is_node_id = inner
        .split('.')
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()));
    if is_node_id {
        rest
    } else {
        title
    }
}

/// Branch-safe slug: lowercase ASCII letters, digits and single dashes, at
/// most `SLUG_MAX` long (cut at a word boundary), node-id prefix dropped.
/// (`capture::slugify` makes memory file names and keeps every word.)
pub(super) fn branch_slug(title: &str) -> String {
    let kept: String = strip_node_prefix(title)
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == ' ' || *c == '-')
        .collect();
    let mut slug = String::new();
    for c in kept.trim().chars().map(|c| if c == ' ' { '-' } else { c }) {
        if !(c == '-' && slug.ends_with('-')) {
            slug.push(c);
        }
    }
    if slug.len() > SLUG_MAX {
        // ASCII only, so byte indices are char indices.
        let cut = slug[..=SLUG_MAX].rfind('-').unwrap_or(SLUG_MAX);
        slug.truncate(cut);
    }
    slug.trim_end_matches('-').to_string()
}

/// `template` with each `{name}` replaced by its value for `plan`. Every
/// missing `{meta[key]}` is reported at once.
pub(super) fn fill_template(template: &str, plan: &PlanFields) -> Result<String> {
    let bad = |why: String| anyhow::anyhow!("bad template `{template}`: {why}");
    let mut out = String::new();
    let mut missing_meta: Vec<&str> = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after.find('}').ok_or_else(|| bad("unclosed `{`".into()))?;
        let name = &after[..close];
        match name.strip_prefix("meta[").and_then(|r| r.strip_suffix(']')) {
            Some(key) => match plan.meta.get(key) {
                Some(value) => out.push_str(value),
                None if !missing_meta.contains(&key) => missing_meta.push(key),
                None => {}
            },
            None => out.push_str(&placeholder_value(name, plan).ok_or_else(|| {
                bad(format!(
                    "unknown placeholder `{{{name}}}` (expected number, title, slug or meta[key])"
                ))
            })?),
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    if !missing_meta.is_empty() {
        return Err(bad(missing_meta_hint(plan, &missing_meta)));
    }
    Ok(out)
}

/// Which metadata keys plan #N lacks, and how to supply each: kata metadata
/// on an existing mirror, else (the mirror doesn't exist yet) a line in the
/// plan body, which only `jira` and `type` can come from.
pub(super) fn missing_meta_hint(plan: &PlanFields, keys: &[&str]) -> String {
    let quoted: Vec<String> = keys.iter().map(|k| format!("'{k}'")).collect();
    let hints: Vec<String> = keys
        .iter()
        .map(|&key| match (plan.kata_ref.as_deref(), key) {
            (Some(r), _) => format!("kata meta set {r} {key} <value>"),
            (None, kata::JIRA_META_KEY) => "add a `Jira: KEY-1` line to the plan issue body".into(),
            (None, kata::TYPE_META_KEY) => "add a `Type: feat` line to the plan issue body".into(),
            (None, _) => format!(
                "'{key}' only comes from kata metadata: mirror the plan in kata, then \
                 `kata meta set <kata-ref> {key} <value>`"
            ),
        })
        .collect();
    format!(
        "plan #{} has no metadata {} ({})",
        plan.number,
        quoted.join(", "),
        hints.join("; ")
    )
}

/// The value of a non-meta placeholder, or `None` if `name` isn't one.
pub(super) fn placeholder_value(name: &str, plan: &PlanFields) -> Option<String> {
    match name {
        "number" => Some(plan.number.to_string()),
        "title" => Some(plan.title.clone()),
        "slug" => Some(branch_slug(&plan.title)),
        _ => None,
    }
}

/// `fill_template` for `[dispatch] pr_title`, refusing a blank title that
/// `gh pr create` would only reject after the branch was pushed.
pub(super) fn fill_pr_title(template: &str, plan: &PlanFields) -> Result<String> {
    let title = fill_template(template, plan)?;
    if title.trim().is_empty() {
        anyhow::bail!(
            "[dispatch] pr_title `{template}` renders an empty PR title for plan #{}",
            plan.number
        );
    }
    Ok(title)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::test_support::*;
    use rstest::rstest;

    #[rstest]
    #[case::all_placeholders(
        "{meta[type]}/{meta[jira]}/{number}-{slug}: {title}",
        Some("a56d"),
        Ok("feat/ENG-7/142-add-dispatch-preflight: Add dispatch preflight!")
    )]
    #[case::stray_close_brace_is_literal("{number}}", None, Ok("142}"))]
    #[case::missing_meta(
        "{meta[team]}-{meta[jira]}-{meta[area]}-{meta[team]}",
        Some("a56d"),
        Err(
            "plan #142 has no metadata 'team', 'area' \
             (kata meta set a56d team <value>; kata meta set a56d area <value>)"
        )
    )]
    #[case::missing_meta_without_mirror(
        "{meta[team]}",
        None,
        Err("'team' only comes from kata metadata: mirror the plan in kata")
    )]
    #[case::kata_ref_is_gone("{kata_ref}", Some("a56d"), Err("unknown placeholder `{kata_ref}`"))]
    #[case::unknown_placeholder("{short_id}-{slug}", None, Err("unknown placeholder `{short_id}`"))]
    #[case::unclosed("{number", None, Err("unclosed `{`"))]
    fn fill_template_fills_placeholders_or_explains(
        #[case] template: &str,
        #[case] kata_ref: Option<&str>,
        #[case] want: std::result::Result<&str, &str>,
    ) {
        match (fill_template(template, &plan(kata_ref)), want) {
            (Ok(got), Ok(want)) => assert_eq!(got, want),
            (Err(e), Err(part)) => assert!(e.to_string().contains(part), "{e}"),
            (got, want) => panic!("got {got:?}, want {want:?}"),
        }
    }

    // Without a mirror, jira and type can only come from the plan body.
    #[rstest]
    #[case::mirror(
        Some("a56d"),
        &["no metadata 'jira', 'type' (kata meta set a56d jira <value>; kata meta set a56d type"],
        &["issue body"]
    )]
    #[case::no_mirror(
        None,
        &["`Jira: KEY-1` line to the plan issue body; add a `Type: feat` line"],
        &["kata meta set"]
    )]
    fn fill_template_hints_how_to_supply_missing_meta(
        #[case] kata_ref: Option<&str>,
        #[case] has: &[&str],
        #[case] lacks: &[&str],
    ) {
        let p = PlanFields {
            meta: BTreeMap::new(),
            ..plan(kata_ref)
        };
        let err = fill_template("{meta[jira]}/{meta[type]}", &p).unwrap_err();
        assert_mentions(&format!("{err:#}"), has, lacks);
    }

    #[rstest]
    #[case::filled("{title}", Ok("Add dispatch preflight!"))]
    #[case::empty("", Err("[dispatch] pr_title ``"))]
    #[case::whitespace("  ", Err("renders an empty PR title for plan #142"))]
    fn fill_pr_title_refuses_blank_titles(#[case] template: &str, #[case] want: Want<&str>) {
        assert_result(fill_pr_title(template, &plan(None)), want.map(Into::into));
    }

    #[rstest]
    #[case::punctuation_and_dash_runs("Fix: the -- login  race!", "fix-the-login-race")]
    #[case::nothing_left("!!! ??? ...", "")]
    #[case::node_prefix_dropped_and_cut_at_a_word(
        "[5.1] Add dispatch preflight, [dispatch] config and doctor check",
        "add-dispatch-preflight-dispatch-config"
    )]
    #[case::numeric_node_prefix_dropped("[12] Fix", "fix")]
    #[case::other_brackets_kept("[ENG-7] Fix login", "eng-7-fix-login")]
    // An empty id part means it isn't a node id.
    #[case::not_a_node_id("[5.] Fix", "5-fix")]
    #[case::one_long_word_cut_hard(&"x".repeat(50), &"x".repeat(SLUG_MAX))]
    fn branch_slug_cases(#[case] title: &str, #[case] slug: &str) {
        assert_eq!(branch_slug(title), slug);
    }
}
