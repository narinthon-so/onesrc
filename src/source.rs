//! The source directory: `onesrc.toml`, `AGENTS.md`, `fragments/`, `skills/`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentId {
    Claude,
    Codex,
    Agy,
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AgentId::Claude => "claude",
            AgentId::Codex => "codex",
            AgentId::Agy => "agy",
        })
    }
}

/// A string from the source: either literal, or a whole-string `${VAR}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Literal(String),
    EnvRef(String),
}

impl Value {
    fn parse(field: &str, raw: &str) -> Result<Self> {
        if let Some(name) = raw.strip_prefix("${").and_then(|r| r.strip_suffix('}')) {
            if is_env_name(name) {
                return Ok(Value::EnvRef(name.to_string()));
            }
        }
        if raw.contains("${") {
            bail!("{field}: only a whole-string ${{VAR}} is supported, got {raw:?}");
        }
        Ok(Value::Literal(raw.to_string()))
    }
}

fn is_env_name(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, Value>,
    },
    Http {
        url: String,
        headers: BTreeMap<String, Value>,
        /// Name of the environment variable holding the bearer token.
        bearer_token_env: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct McpServer {
    pub name: String,
    pub transport: Transport,
    pub targets: Option<Vec<AgentId>>,
    pub enabled: bool,
}

impl McpServer {
    pub fn targets(&self, agent: AgentId) -> bool {
        self.targets.as_ref().is_none_or(|t| t.contains(&agent))
    }
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub dir: PathBuf,
}

#[derive(Debug)]
pub struct Source {
    pub root: PathBuf,
    pub mcp: Vec<McpServer>,
    pub skills: Vec<Skill>,
    instructions: Option<PathBuf>,
    fragments: BTreeMap<AgentId, PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    mcp: BTreeMap<String, RawMcp>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMcp {
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    url: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    bearer_token: Option<String>,
    targets: Option<Vec<AgentId>>,
    #[serde(default = "default_true")]
    enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Source {
    pub fn load(root: &Path, home: &Path) -> Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("source directory {} not found", root.display()))?;
        let toml_path = root.join("onesrc.toml");
        let mcp = if toml_path.exists() {
            let text = std::fs::read_to_string(&toml_path)
                .with_context(|| format!("reading {}", toml_path.display()))?;
            let raw: RawFile = toml::from_str(&text).context("parsing onesrc.toml")?;
            raw.mcp
                .into_iter()
                .map(|(name, m)| McpServer::from_raw(name, m, home, &root))
                .collect::<Result<_>>()?
        } else {
            Vec::new()
        };

        let instructions = Some(root.join("AGENTS.md")).filter(|p| p.is_file());
        let mut fragments = BTreeMap::new();
        for agent in [AgentId::Claude, AgentId::Codex, AgentId::Agy] {
            let p = root.join("fragments").join(format!("{agent}.md"));
            if p.is_file() {
                fragments.insert(agent, p);
            }
        }

        let mut skills = Vec::new();
        let skills_dir = root.join("skills");
        if skills_dir.is_dir() {
            for entry in std::fs::read_dir(&skills_dir)? {
                let entry = entry?;
                let dir = entry.path();
                if dir.join("SKILL.md").is_file() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    skills.push(Skill { name, dir });
                }
            }
            skills.sort_by(|a, b| a.name.cmp(&b.name));
        }

        Ok(Self {
            root,
            mcp,
            skills,
            instructions,
            fragments,
        })
    }

    /// Rendered instructions for one agent: `AGENTS.md`, then the agent's
    /// fragment if there is one. `None` when the source has no `AGENTS.md`.
    pub fn instructions_for(&self, agent: AgentId) -> Result<Option<String>> {
        let Some(base) = &self.instructions else {
            return Ok(None);
        };
        let mut text = std::fs::read_to_string(base)?;
        if let Some(frag) = self.fragments.get(&agent) {
            if !text.ends_with('\n') {
                text.push('\n');
            }
            text.push('\n');
            text.push_str(&std::fs::read_to_string(frag)?);
        }
        Ok(Some(text))
    }

    /// True when the agent's instructions are exactly `AGENTS.md`, so a
    /// symlink to it is a valid installation.
    pub fn instructions_linkable(&self, agent: AgentId) -> Option<&Path> {
        match self.fragments.contains_key(&agent) {
            true => None,
            false => self.instructions.as_deref(),
        }
    }
}

impl McpServer {
    fn from_raw(name: String, m: RawMcp, home: &Path, source: &Path) -> Result<Self> {
        let field = |f: &str| format!("mcp.{name}.{f}");
        let transport = match (m.command, m.url) {
            (Some(command), None) => {
                if !m.headers.is_empty() || m.bearer_token.is_some() {
                    bail!("{}: headers/bearer_token need url", field("command"));
                }
                let command = substitute(&field("command"), &command, home, source)?;
                let args = m
                    .args
                    .iter()
                    .map(|a| substitute(&field("args"), a, home, source))
                    .collect::<Result<_>>()?;
                let env = m
                    .env
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), Value::parse(&field("env"), v)?)))
                    .collect::<Result<_>>()?;
                Transport::Stdio { command, args, env }
            }
            (None, Some(url)) => {
                if !m.args.is_empty() || !m.env.is_empty() {
                    bail!("{}: args/env need command", field("url"));
                }
                let headers = m
                    .headers
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), Value::parse(&field("headers"), v)?)))
                    .collect::<Result<_>>()?;
                let bearer_token_env = match m.bearer_token {
                    None => None,
                    Some(raw) => match Value::parse(&field("bearer_token"), &raw)? {
                        Value::EnvRef(var) => Some(var),
                        Value::Literal(_) => {
                            bail!(
                                "{}: must be ${{VAR}}, never a literal",
                                field("bearer_token")
                            )
                        }
                    },
                };
                Transport::Http {
                    url,
                    headers,
                    bearer_token_env,
                }
            }
            (Some(_), Some(_)) => bail!("mcp.{name}: set command or url, not both"),
            (None, None) => bail!("mcp.{name}: needs command or url"),
        };
        Ok(Self {
            name,
            transport,
            targets: m.targets,
            enabled: m.enabled,
        })
    }
}

/// Apply `{home}` / `{source}`; reject a leading `~`, which no process expands.
fn substitute(field: &str, raw: &str, home: &Path, source: &Path) -> Result<String> {
    if raw == "~" || raw.starts_with("~/") {
        bail!("{field}: {raw:?} starts with ~, which is not expanded; use {{home}}");
    }
    Ok(raw
        .replace("{home}", &home.to_string_lossy())
        .replace("{source}", &source.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_values() {
        assert_eq!(
            Value::parse("f", "${A_1}").unwrap(),
            Value::EnvRef("A_1".into())
        );
        assert_eq!(
            Value::parse("f", "plain").unwrap(),
            Value::Literal("plain".into())
        );
        assert!(Value::parse("f", "Bearer ${TOKEN}").is_err());
        assert!(Value::parse("f", "${1BAD}").is_err());
    }

    #[test]
    fn tilde_rejected() {
        let h = Path::new("/h");
        let s = Path::new("/s");
        assert!(substitute("f", "~/x", h, s).is_err());
        assert_eq!(substitute("f", "{home}/x", h, s).unwrap(), "/h/x");
        assert_eq!(substitute("f", "a~b", h, s).unwrap(), "a~b");
    }
}
