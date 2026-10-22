//! `engram dispatch`: hand plan issues to background Claude sessions.
//!
//! Dispatch is the only engram command that needs kwt, tmux and kata, so it
//! checks for them up front (`preflight`) instead of degrading silently.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// kwt is often only a shell alias to the copy bundled with Ghosthub, which
/// `Command::new("kwt")` can't see.
const GHOSTHUB_KWT: &str = "/Applications/Ghosthub.app/Contents/Helpers/kwt";
const TOOLS: &[(&str, &str)] = &[
    ("gh", "--version"),
    ("kata", "--version"),
    ("kwt", "--version"),
    ("tmux", "-V"),
    ("claude", "--version"),
];
const SLUG_MAX: usize = 40;
const PENDING_KATA_REF: &str = "<kata-ref>";

/// Paths of the tools dispatch runs itself (gh and kata go through github.rs
/// and kata.rs by name; preflight only checks that they work).
#[derive(Debug)]
pub struct Toolchain {
    kwt: PathBuf,
    tmux: PathBuf,
    claude: PathBuf,
}

fn resolve_tool(name: &str, path_var: Option<&OsStr>, fallback: Option<&Path>) -> Option<PathBuf> {
    path_var
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(name))
        .chain(fallback.map(Path::to_path_buf))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn probe(repo_root: &Path) -> (BTreeMap<&'static str, PathBuf>, Vec<String>) {
    let path_var = std::env::var_os("PATH");
    let mut found = BTreeMap::new();
    let mut missing = Vec::new();
    for &(name, flag) in TOOLS {
        let fallback = (name == "kwt").then(|| Path::new(GHOSTHUB_KWT));
        let ok = |p: &PathBuf| {
            Command::new(p)
                .arg(flag)
                .output()
                .is_ok_and(|o| o.status.success())
        };
        match resolve_tool(name, path_var.as_deref(), fallback).filter(ok) {
            Some(p) => {
                found.insert(name, p);
            }
            None => missing.push(name),
        }
    }
    let kata_bound = repo_root.join(".kata.toml").exists();
    let why = reasons(&missing, found.contains_key("kata"), kata_bound);
    (found, why)
}

fn reasons(missing: &[&str], kata_installed: bool, kata_bound: bool) -> Vec<String> {
    let mut out = Vec::new();
    if !missing.is_empty() {
        out.push(format!("missing: {}", missing.join(", ")));
    }
    // A missing kata binary is reported above; flag the binding only when kata
    // itself is installed, so the fix is unambiguous.
    if kata_installed && !kata_bound {
        out.push("repo not bound to kata, run `kata init`".to_string());
    }
    out
}

pub fn unavailable_reasons(repo_root: &Path) -> Vec<String> {
    probe(repo_root).1
}

pub fn preflight(repo_root: &Path) -> Result<Toolchain> {
    let (mut found, problems) = probe(repo_root);
    if !problems.is_empty() {
        anyhow::bail!("engram dispatch can't run: {}", problems.join("; "));
    }
    let mut take = |name: &str| {
        found
            .remove(name)
            .with_context(|| format!("{name} resolved but not recorded"))
    };
    Ok(Toolchain {
        kwt: take("kwt")?,
        tmux: take("tmux")?,
        claude: take("claude")?,
    })
}

#[derive(Debug, Clone, Default)]
struct TemplateVars {
    number: u64,
    kata_ref: Option<String>,
    title: String,
    meta: BTreeMap<String, String>,
}

/// `title` without a leading node-id bracket such as `[5.1] `; other bracket
/// groups (`[ENG-7] ...`) stay.
fn strip_node_prefix(title: &str) -> &str {
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
fn slugify(title: &str) -> String {
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

fn render(template: &str, vars: &TemplateVars) -> Result<String> {
    let bad = |why: String| anyhow::anyhow!("bad template `{template}`: {why}");
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' | '}' if chars.peek() == Some(&c) => {
                chars.next();
                out.push(c);
            }
            '}' => return Err(bad("unmatched `}` (use `}}` for a literal brace)".into())),
            '{' => {
                let mut name = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some('{') | None => {
                            return Err(bad("unclosed `{` (use `{{` for a literal brace)".into()))
                        }
                        Some(ch) => name.push(ch),
                    }
                }
                out.push_str(&expand(&name, vars).map_err(bad)?);
            }
            c => out.push(c),
        }
    }
    Ok(out)
}

fn expand(name: &str, vars: &TemplateVars) -> std::result::Result<String, String> {
    let n = vars.number;
    if let Some(key) = name.strip_prefix("meta[").and_then(|r| r.strip_suffix(']')) {
        let kata_ref = vars.kata_ref.as_deref().unwrap_or(PENDING_KATA_REF);
        return vars.meta.get(key).cloned().ok_or_else(|| {
            format!(
                "plan #{n} has no kata metadata '{key}' (kata meta set {kata_ref} {key} <value>)"
            )
        });
    }
    match name {
        "number" => Ok(n.to_string()),
        "kata_ref" => vars
            .kata_ref
            .clone()
            .ok_or_else(|| format!("plan #{n} has no kata mirror for {{kata_ref}}")),
        "title" => Ok(vars.title.clone()),
        "slug" => Ok(slugify(&vars.title)),
        other => Err(format!(
            "unknown placeholder `{{{other}}}` (expected number, kata_ref, title, slug or meta[key])"
        )),
    }
}

/// Render before the plan may have a mirror: a `{kata_ref}` that doesn't exist
/// yet shows as `PENDING_KATA_REF`, and the result is flagged pending.
fn render_planned(template: &str, vars: &TemplateVars) -> Result<(String, bool)> {
    if vars.kata_ref.is_some() {
        return Ok((render(template, vars)?, false));
    }
    let shown = TemplateVars {
        kata_ref: Some(PENDING_KATA_REF.to_string()),
        ..vars.clone()
    };
    let text = render(template, &shown)?;
    // Only kata_ref differs, so a failure here means the template uses it.
    Ok((text, render(template, vars).is_err()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn vars(kata_ref: Option<&str>) -> TemplateVars {
        TemplateVars {
            number: 142,
            kata_ref: kata_ref.map(Into::into),
            title: "Add dispatch preflight!".into(),
            meta: BTreeMap::from([
                ("jira".into(), "ENG-7".into()),
                ("type".into(), "feat".into()),
            ]),
        }
    }

    /// The expected value, or text the error's full message contains.
    type Want<T> = std::result::Result<T, &'static str>;

    fn assert_result<T: PartialEq + std::fmt::Debug>(got: Result<T>, want: Want<T>) {
        match (got, want) {
            (Ok(got), Ok(want)) => assert_eq!(got, want),
            (Err(e), Err(part)) => assert!(format!("{e:#}").contains(part), "{e:#}"),
            (got, want) => panic!("got {got:?}, want {want:?}"),
        }
    }

    #[rstest]
    #[case::all_placeholders(
        "{meta[type]}/{meta[jira]}/{number}-{kata_ref}-{slug}: {title}",
        Some("a56d"),
        Ok("feat/ENG-7/142-a56d-add-dispatch-preflight: Add dispatch preflight!")
    )]
    #[case::escaped_braces("{{x}} {number}}}", None, Ok("{x} 142}"))]
    #[case::missing_meta("{meta[team]}", Some("a56d"), Err("(kata meta set a56d team <value>)"))]
    #[case::missing_meta_and_mirror("{meta[team]}", None, Err("kata meta set <kata-ref> team"))]
    #[case::missing_mirror("{kata_ref}", None, Err("no kata mirror"))]
    #[case::unknown_placeholder("{short_id}-{slug}", None, Err("unknown placeholder `{short_id}`"))]
    #[case::unclosed("{number", None, Err("unclosed `{`"))]
    #[case::nested("{num{ber}", None, Err("unclosed `{`"))]
    #[case::unmatched_close("number}", None, Err("unmatched `}`"))]
    fn render_fills_placeholders_or_explains(
        #[case] template: &str,
        #[case] kata_ref: Option<&str>,
        #[case] want: Want<&str>,
    ) {
        assert_result(render(template, &vars(kata_ref)), want.map(Into::into));
    }

    #[rstest]
    #[case::pending("{kata_ref}-{slug}", None, Ok(("<kata-ref>-add-dispatch-preflight", true)))]
    #[case::unused("{number}-{slug}", None, Ok(("142-add-dispatch-preflight", false)))]
    #[case::present("{kata_ref}", Some("a56d"), Ok(("a56d", false)))]
    #[case::other_errors_still_fail("{kata_ref}-{meta[team]}", None, Err("metadata 'team'"))]
    fn render_planned_marks_missing_kata_ref_pending(
        #[case] template: &str,
        #[case] kata_ref: Option<&str>,
        #[case] want: Want<(&str, bool)>,
    ) {
        let got = render_planned(template, &vars(kata_ref));
        assert_result(got, want.map(|(text, pending)| (text.into(), pending)));
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
    fn slugify_cases(#[case] title: &str, #[case] slug: &str) {
        assert_eq!(slugify(title), slug);
    }

    #[cfg(unix)]
    #[test]
    fn resolve_tool_prefers_path_then_executable_fallback() {
        use std::os::unix::fs::PermissionsExt;
        let exe = |dir: &Path| {
            let p = dir.join("kwt");
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        let [bin, other, empty] = [(); 3].map(|_| tempfile::tempdir().unwrap());
        let (on_path, fallback) = (exe(bin.path()), exe(other.path()));
        let path = |d: &tempfile::TempDir| std::env::join_paths([d.path()]).unwrap();
        let kwt = |d| resolve_tool("kwt", Some(&path(d)), Some(&fallback));
        assert_eq!(kwt(&bin), Some(on_path));
        assert_eq!(kwt(&empty), Some(fallback.clone()));
        // Present but not executable, and no fallback.
        std::fs::write(empty.path().join("tmux"), "").unwrap();
        assert_eq!(resolve_tool("tmux", Some(&path(&empty)), None), None);
    }

    #[rstest]
    #[case::missing_tools(&["kwt", "tmux"], true, true, &["missing: kwt, tmux"])]
    #[case::unbound(&[], true, false, &["repo not bound to kata, run `kata init`"])]
    #[case::binding_unchecked_without_kata(&["kata"], false, false, &["missing: kata"])]
    #[case::ready(&[], true, true, &[])]
    fn reasons_cases(
        #[case] missing: &[&str],
        #[case] kata_installed: bool,
        #[case] kata_bound: bool,
        #[case] want: &[&str],
    ) {
        assert_eq!(reasons(missing, kata_installed, kata_bound), want);
    }
}
