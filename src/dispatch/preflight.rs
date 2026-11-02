//! Finding the tools dispatch runs (kwt, tmux, claude) and checking that
//! gh and kata work, before anything is changed.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::integrations::kata;

/// kwt is often only a shell alias to the copy bundled with Ghosthub, which
/// `Command::new("kwt")` can't see.
pub(super) const GHOSTHUB_KWT: &str = "/Applications/Ghosthub.app/Contents/Helpers/kwt";
pub(super) const TOOLS: &[(&str, &str)] = &[
    ("gh", "--version"),
    ("kata", "--version"),
    ("kwt", "--version"),
    ("tmux", "-V"),
    ("claude", "--version"),
];
/// Paths of the tools dispatch runs itself (gh and kata go through github.rs
/// and kata.rs by name; preflight only checks that they work).
#[derive(Debug)]
pub struct Toolchain {
    pub(super) kwt: PathBuf,
    pub(super) tmux: PathBuf,
    pub(super) claude: PathBuf,
}

pub(super) fn resolve_tool(
    name: &str,
    path_var: Option<&OsStr>,
    fallback: Option<&Path>,
) -> Option<PathBuf> {
    path_var
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(name))
        .chain(fallback.map(Path::to_path_buf))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
pub(super) fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
pub(super) fn is_executable(path: &Path) -> bool {
    path.is_file()
}

pub(super) fn check_tools(repo_root: &Path) -> (BTreeMap<&'static str, PathBuf>, Vec<String>) {
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
    let kata_bound = repo_root.join(kata::KATA_CONFIG).exists();
    let problems = tool_problems(&missing, found.contains_key("kata"), kata_bound);
    (found, problems)
}

pub(super) fn tool_problems(
    missing: &[&str],
    kata_installed: bool,
    kata_bound: bool,
) -> Vec<String> {
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
    check_tools(repo_root).1
}

pub fn preflight(repo_root: &Path) -> Result<Toolchain> {
    let (mut found, problems) = check_tools(repo_root);
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

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

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
        let [bin, other, empty] = [(); 3].map(|()| tempfile::tempdir().unwrap());
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
    fn tool_problems_cases(
        #[case] missing: &[&str],
        #[case] kata_installed: bool,
        #[case] kata_bound: bool,
        #[case] want: &[&str],
    ) {
        assert_eq!(tool_problems(missing, kata_installed, kata_bound), want);
    }
}
