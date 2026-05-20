use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    pub github: GitHubConfig,
    pub memory: MemoryConfig,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct GitHubConfig {
    pub repo: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryConfig {
    pub default_categories: Vec<String>,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            default_categories: vec![
                "patterns".into(),
                "tripwires".into(),
                "architecture".into(),
                "testing".into(),
            ],
        }
    }
}

impl Config {
    pub fn load(repo_root: &Path) -> Result<Self> {
        let path = repo_root.join(".engram/config.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&contents).context("parsing .engram/config.toml")
    }

    pub fn save(&self, repo_root: &Path) -> Result<()> {
        let dir = repo_root.join(".engram");
        std::fs::create_dir_all(&dir)?;
        let contents = toml::to_string_pretty(self)?;
        std::fs::write(dir.join("config.toml"), contents)?;
        Ok(())
    }

    pub fn repo(&self) -> Option<&str> {
        self.github.repo.as_deref()
    }
}

pub fn find_repo_root() -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("running git rev-parse")?;
    if !output.status.success() {
        anyhow::bail!("not in a git repository");
    }
    let path = String::from_utf8(output.stdout)?.trim().to_string();
    Ok(PathBuf::from(path))
}

pub fn resolve_repo(cfg: &Config, repo_root: &Path) -> Result<String> {
    if let Some(repo) = cfg.repo() {
        return Ok(repo.to_string());
    }
    let output = Command::new("gh")
        .args([
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "-q",
            ".nameWithOwner",
        ])
        .current_dir(repo_root)
        .output()?;
    if output.status.success() {
        return Ok(String::from_utf8(output.stdout)?.trim().to_string());
    }
    anyhow::bail!("GitHub repo not configured — run `engram init`")
}
