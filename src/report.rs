//! Output contracts: the probe report and status records (both versioned).

use std::path::Path;

use serde::Serialize;

use crate::ctx::Ctx;
use crate::source::AgentId;

pub const SCHEMA_VERSION: u32 = 1;

// ---------- probe ----------

#[derive(Serialize)]
pub struct ProbeReport {
    pub schema_version: u32,
    pub agent: AgentInfo,
    pub platform: Platform,
    pub paths: Vec<PathInfo>,
    pub checks: Vec<Check>,
    pub redacted: bool,
}

#[derive(Serialize)]
pub struct AgentInfo {
    pub id: AgentId,
    pub executable: Option<String>,
    pub version: Option<String>,
}

#[derive(Serialize)]
pub struct Platform {
    pub os: &'static str,
    pub arch: &'static str,
    pub wsl: bool,
}

impl Platform {
    pub fn current() -> Self {
        let wsl = std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|s| s.to_ascii_lowercase().contains("microsoft"))
            .unwrap_or(false);
        Self {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            wsl,
        }
    }
}

#[derive(Serialize)]
pub struct PathInfo {
    pub role: &'static str,
    pub path: String,
    #[serde(flatten)]
    pub kind: PathKind,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PathKind {
    Missing,
    File,
    Dir,
    Symlink { target: String },
    DanglingSymlink { target: String },
    Other,
}

impl PathKind {
    pub fn of(ctx: &Ctx, path: &Path) -> Self {
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return PathKind::Missing;
        };
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(path)
                .map(|t| ctx.show(&t))
                .unwrap_or_default();
            return match path.exists() {
                true => PathKind::Symlink { target },
                false => PathKind::DanglingSymlink { target },
            };
        }
        if meta.is_dir() {
            PathKind::Dir
        } else if meta.is_file() {
            PathKind::File
        } else {
            PathKind::Other
        }
    }
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckResult {
    Supported,
    Unsupported,
    Unknown,
    Error,
}

/// Where a check's answer comes from. Documentation is kept apart from
/// observation so a doc claim is never mistaken for a probe result.
#[derive(Serialize, Clone, Copy, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    Observed,
    Documented,
}

#[derive(Serialize)]
pub struct Check {
    pub id: &'static str,
    pub result: CheckResult,
    pub evidence: Option<Evidence>,
    pub detail: String,
}

impl Check {
    pub fn new(
        id: &'static str,
        result: CheckResult,
        evidence: Option<Evidence>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            id,
            result,
            evidence,
            detail: detail.into(),
        }
    }
}

pub fn render_probe(r: &ProbeReport) -> String {
    let mut out = format!(
        "{} — executable: {}, version: {}\n",
        r.agent.id,
        r.agent.executable.as_deref().unwrap_or("not found"),
        r.agent.version.as_deref().unwrap_or("unknown"),
    );
    out.push_str("paths:\n");
    for p in &r.paths {
        let kind = match &p.kind {
            PathKind::Missing => "missing".to_string(),
            PathKind::File => "file".to_string(),
            PathKind::Dir => "dir".to_string(),
            PathKind::Symlink { target } => format!("symlink -> {target}"),
            PathKind::DanglingSymlink { target } => format!("DANGLING symlink -> {target}"),
            PathKind::Other => "other".to_string(),
        };
        out.push_str(&format!("  {:<22} {}  ({kind})\n", p.role, p.path));
    }
    out.push_str("checks:\n");
    for c in &r.checks {
        let result = serde_json::to_value(c.result).unwrap();
        let evidence = c
            .evidence
            .map(|e| {
                serde_json::to_value(e)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .unwrap_or_else(|| "-".into());
        out.push_str(&format!(
            "  {:<34} {:<11} [{evidence}] {}\n",
            c.id,
            result.as_str().unwrap(),
            c.detail
        ));
    }
    out
}

// ---------- status ----------

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    InSync,
    Missing,
    Differs,
    Unmanaged,
    Conflict,
    Shadowed,
    Unknown,
}

impl State {
    /// States that make `status` exit non-zero.
    pub fn is_drift(self) -> bool {
        !matches!(self, State::InSync | State::Unmanaged)
    }
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Mcp,
    Skill,
    Instructions,
}

#[derive(Serialize)]
pub struct StatusRecord {
    pub agent: AgentId,
    pub kind: ItemKind,
    pub item: String,
    pub path: String,
    pub state: State,
    /// Ownership comes from `state.json` (M1). Until then nothing is owned.
    pub owned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Serialize)]
pub struct StatusReport {
    pub schema_version: u32,
    pub records: Vec<StatusRecord>,
}

pub fn render_status(r: &StatusReport) -> String {
    let mut out = String::new();
    for rec in &r.records {
        let state = serde_json::to_value(rec.state).unwrap();
        let kind = serde_json::to_value(rec.kind).unwrap();
        out.push_str(&format!(
            "{:<7} {:<12} {:<12} {:<20} {}",
            rec.agent.to_string(),
            kind.as_str().unwrap(),
            state.as_str().unwrap(),
            rec.item,
            rec.path
        ));
        if let Some(d) = &rec.detail {
            out.push_str(&format!("\n{:>34}↳ {d}", ""));
        }
        out.push('\n');
    }
    let drift = r.records.iter().filter(|r| r.state.is_drift()).count();
    out.push_str(&format!(
        "{} item(s), {drift} need attention\n",
        r.records.len()
    ));
    out
}
