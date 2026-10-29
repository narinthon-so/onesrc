//! Process context: where "home" is and what the environment says.
//!
//! Nothing below `main` reads the real environment directly, so tests can
//! point every command at a disposable fixture home.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

pub struct Ctx {
    pub home: PathBuf,
    pub source: Option<PathBuf>,
    env: BTreeMap<String, String>,
}

impl Ctx {
    pub fn from_process(source: Option<PathBuf>) -> Result<Self> {
        let env: BTreeMap<String, String> = std::env::vars().collect();
        let home = env
            .get("HOME")
            .map(PathBuf::from)
            .context("HOME is not set")?;
        Ok(Self { home, source, env })
    }

    pub fn env(&self, key: &str) -> Option<&str> {
        self.env
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    /// Path for display: `{source}/…` or `{home}/…` instead of the absolute
    /// path, so output never leaks a username and snapshots stay stable.
    pub fn show(&self, path: &Path) -> String {
        if let Some(src) = &self.source {
            if let Ok(rest) = path.strip_prefix(src) {
                return join_display("{source}", rest);
            }
        }
        if let Ok(rest) = path.strip_prefix(&self.home) {
            return join_display("{home}", rest);
        }
        path.display().to_string()
    }

    /// Rewrite a leading home path into source syntax (`{home}/…`).
    pub fn to_source_syntax(&self, s: &str) -> String {
        let home = self.home.to_string_lossy();
        match s.strip_prefix(home.as_ref()) {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("{{home}}{rest}"),
            _ => s.to_string(),
        }
    }

    /// Find an executable on this context's `PATH`.
    pub fn which(&self, name: &str) -> Option<PathBuf> {
        let path = self.env("PATH")?;
        std::env::split_paths(path)
            .map(|dir| dir.join(name))
            .find(|p| p.is_file())
    }
}

fn join_display(prefix: &str, rest: &Path) -> String {
    if rest.as_os_str().is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}/{}", rest.display())
    }
}
