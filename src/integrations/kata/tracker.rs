//! Ticket metadata (`jira`, `type`) for kata mirrors, read from CLI flags,
//! `Jira:`/`Type:` body lines or the issue title.

use std::collections::BTreeMap;

pub const JIRA_META_KEY: &str = "jira";
pub const TYPE_META_KEY: &str = "type";
const CHANGE_TYPES: &[&str] = &[
    "feat", "fix", "chore", "docs", "refactor", "test", "perf", "build", "ci", "style", "revert",
];

/// Ticket metadata that `engram dispatch`'s `[dispatch]` branch/PR-title templates read from kata
/// (`{meta[jira]}`, `{meta[type]}`). Absent fields are never written — no placeholders.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackerMeta {
    pub jira: Option<String>,
    pub kind: Option<String>,
}

impl TrackerMeta {
    /// Explicit flags win; otherwise read `Jira:` / `Type:` lines from the body,
    /// then fall back to the title (`[KEY-12] ...`, `KEY-12: ...`, `feat: ...`).
    pub fn resolve(
        jira_flag: Option<&str>,
        type_flag: Option<&str>,
        title: &str,
        body: &str,
    ) -> Self {
        let jira = jira_flag
            .map(str::to_string)
            .or_else(|| body_field(body, JIRA_META_KEY).and_then(jira_key_from_value))
            .or_else(|| jira_key_from_title(title));
        let kind = type_flag
            .map(str::to_string)
            .or_else(|| body_field(body, TYPE_META_KEY).and_then(change_type))
            .or_else(|| change_type_from_title(title));
        TrackerMeta { jira, kind }
    }

    /// [`TrackerMeta::resolve`] with these fields as the flags.
    pub fn resolve_for(&self, title: &str, body: &str) -> Self {
        Self::resolve(self.jira.as_deref(), self.kind.as_deref(), title, body)
    }

    /// Fill any missing field from `fallback` (e.g. a child inheriting its objective's key).
    pub fn or(self, fallback: TrackerMeta) -> Self {
        TrackerMeta {
            jira: self.jira.or(fallback.jira),
            kind: self.kind.or(fallback.kind),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.jira.is_some() && self.kind.is_some()
    }

    /// The metadata a mirror created with these fields would carry, keyed as kata stores it.
    pub fn to_map(&self) -> BTreeMap<String, String> {
        self.pairs()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    pub(super) fn from_metadata(metadata: &serde_json::Value) -> Self {
        let field = |key: &str| {
            metadata
                .get(key)
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        TrackerMeta {
            jira: field(JIRA_META_KEY),
            kind: field(TYPE_META_KEY),
        }
    }

    /// `key=value` pairs for `kata create --meta`.
    pub(super) fn create_meta_args(&self) -> Vec<String> {
        self.pairs().map(|(k, v)| format!("{k}={v}")).collect()
    }

    fn pairs(&self) -> impl Iterator<Item = (&'static str, &str)> {
        [(JIRA_META_KEY, &self.jira), (TYPE_META_KEY, &self.kind)]
            .into_iter()
            .filter_map(|(k, v)| v.as_deref().map(|v| (k, v)))
    }
}

/// clap value parser for `--jira`: accepts `KEY-123` (any case) or a browse URL.
pub fn parse_jira_arg(s: &str) -> std::result::Result<String, String> {
    jira_key_from_value(&s.trim().to_uppercase())
        .ok_or_else(|| format!("`{s}` is not a Jira key (expected e.g. ABC-123)"))
}

/// clap value parser for `--type`: one of the conventional-commit change types.
pub fn parse_type_arg(s: &str) -> std::result::Result<String, String> {
    change_type(&s.trim().to_lowercase()).ok_or_else(|| {
        format!(
            "`{s}` is not a change type (one of {})",
            CHANGE_TYPES.join(", ")
        )
    })
}

/// Value of a `Name: value` line (case-insensitive name, optional list bullet / bold).
fn body_field<'a>(body: &'a str, name: &str) -> Option<&'a str> {
    body.lines().find_map(|line| {
        let line = line.trim().trim_start_matches(['-', '*', ' ']);
        let (key, value) = line.split_once(':')?;
        key.trim_matches(['*', ' '])
            .eq_ignore_ascii_case(name)
            .then(|| value.trim_matches(['*', ' ']))
    })
}

fn is_jira_key(s: &str) -> bool {
    let Some((project, number)) = s.split_once('-') else {
        return false;
    };
    let mut chars = project.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && project.len() >= 2
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

/// A bare key, or the last path segment of a Jira browse URL.
fn jira_key_from_value(value: &str) -> Option<String> {
    let token = value.split_whitespace().next()?;
    let candidate = token.trim_end_matches('/').rsplit('/').next()?;
    is_jira_key(candidate).then(|| candidate.to_string())
}

/// Only keys in deliberate positions count — a bracketed `[KEY-12]` anywhere or a
/// leading `KEY-12` token — so words like `UTF-8` mid-title are not mistaken for keys.
fn jira_key_from_title(title: &str) -> Option<String> {
    let bracketed = title
        .split('[')
        .skip(1)
        .filter_map(|s| s.split_once(']').map(|(inner, _)| inner.trim()))
        .find(|inner| is_jira_key(inner));
    let leading = title
        .split_whitespace()
        .next()
        .map(|t| t.trim_end_matches(':'))
        .filter(|t| is_jira_key(t));
    bracketed.or(leading).map(str::to_string)
}

fn change_type(value: &str) -> Option<String> {
    let token = value.split_whitespace().next()?;
    CHANGE_TYPES.contains(&token).then(|| token.to_string())
}

/// Conventional-commit prefix: `feat: ...`, `fix(scope): ...`, `chore!: ...`.
fn change_type_from_title(title: &str) -> Option<String> {
    let (prefix, _) = title.trim().split_once(':')?;
    let base = prefix.split('(').next()?.trim_end_matches('!');
    change_type(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(jira: Option<&str>, kind: Option<&str>) -> TrackerMeta {
        TrackerMeta {
            jira: jira.map(String::from),
            kind: kind.map(String::from),
        }
    }

    #[test]
    fn tracker_meta_flags_win_over_body_and_title() {
        let m = TrackerMeta::resolve(
            Some("NGRM-1"),
            Some("fix"),
            "[NGRM-2] feat: thing",
            "Jira: NGRM-3\nType: chore",
        );
        assert_eq!(m, meta(Some("NGRM-1"), Some("fix")));
    }

    #[test]
    fn tracker_meta_reads_body_lines() {
        let body = "## Why\n- **Jira:** https://acme.atlassian.net/browse/NGRM-42\ntype: feat\n";
        let m = TrackerMeta::resolve(None, None, "Some title", body);
        assert_eq!(m, meta(Some("NGRM-42"), Some("feat")));
    }

    #[test]
    fn tracker_meta_reads_title() {
        assert_eq!(
            TrackerMeta::resolve(None, None, "[1.1] [NGRM-7] add thing", ""),
            meta(Some("NGRM-7"), None)
        );
        assert_eq!(
            TrackerMeta::resolve(None, None, "NGRM-8: fix(api): thing", ""),
            meta(Some("NGRM-8"), None)
        );
        assert_eq!(
            TrackerMeta::resolve(None, None, "fix(api)!: thing", ""),
            meta(None, Some("fix"))
        );
    }

    #[test]
    fn tracker_meta_empty_without_ticket() {
        let m = TrackerMeta::resolve(
            None,
            None,
            "Handle UTF-8 and SHA-256 in [1.2]",
            "## Why\nNo ticket here. Typed: nope\nType: banana",
        );
        assert_eq!(m, TrackerMeta::default());
        assert!(m.create_meta_args().is_empty());
    }

    #[test]
    fn tracker_meta_or_fills_only_missing_fields() {
        let m = meta(None, Some("fix")).or(meta(Some("NGRM-1"), Some("feat")));
        assert_eq!(m, meta(Some("NGRM-1"), Some("fix")));
        assert_eq!(m.create_meta_args(), vec!["jira=NGRM-1", "type=fix"]);
        assert_eq!(
            m.to_map(),
            BTreeMap::from([
                ("jira".to_string(), "NGRM-1".to_string()),
                ("type".to_string(), "fix".to_string())
            ])
        );
    }

    #[test]
    fn tracker_meta_from_kata_metadata() {
        let md = serde_json::json!({"jira": "NGRM-9", "engram.sync": {}});
        assert_eq!(TrackerMeta::from_metadata(&md), meta(Some("NGRM-9"), None));
    }

    #[test]
    fn jira_and_type_args_normalize_and_reject() {
        assert_eq!(parse_jira_arg("ngrm-12"), Ok("NGRM-12".to_string()));
        assert!(parse_jira_arg("NGRM").is_err());
        assert!(parse_jira_arg("12-NGRM").is_err());
        assert_eq!(parse_type_arg("Feat"), Ok("feat".to_string()));
        assert!(parse_type_arg("feature").is_err());
    }
}
