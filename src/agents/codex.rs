//! OpenAI Codex CLI adapter (read-only part, M0).
//!
//! Facts marked "documented" come from the Codex docs as reported in the
//! 2026-10-29 design review; none is confirmed by a behavioural probe yet.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{bail, Context as _, Result};

use super::common;
use crate::ctx::Ctx;
use crate::redact::{scrub, MASK};
use crate::report::{
    AgentInfo, Check, CheckResult, Evidence, ItemKind, PathInfo, PathKind, Platform, ProbeReport,
    State, StatusRecord, SCHEMA_VERSION,
};
use crate::source::{AgentId, McpServer, Source, Transport, Value};

const AGENT: AgentId = AgentId::Codex;

struct Paths {
    config: PathBuf,
    instructions: PathBuf,
    instructions_override: PathBuf,
    /// Install root first (documented), then other roots seen in practice.
    skill_roots: Vec<PathBuf>,
}

fn paths(ctx: &Ctx) -> Paths {
    let codex_home = ctx
        .env("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| ctx.home.join(".codex"));
    Paths {
        config: codex_home.join("config.toml"),
        instructions: codex_home.join("AGENTS.md"),
        instructions_override: codex_home.join("AGENTS.override.md"),
        skill_roots: vec![ctx.home.join(".agents/skills"), codex_home.join("skills")],
    }
}

/// One `[mcp_servers.<name>]` table, in Codex's own field names.
#[derive(Debug, Default, PartialEq, Eq)]
struct CodexMcp {
    command: Option<String>,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    env_vars: Vec<String>,
    url: Option<String>,
    http_headers: BTreeMap<String, String>,
    env_http_headers: BTreeMap<String, String>,
    bearer_token_env_var: Option<String>,
    enabled: bool,
    /// Keys onesrc does not model (e.g. `cwd`, timeouts). Not compared.
    other_keys: Vec<String>,
}

impl CodexMcp {
    fn from_table(name: &str, t: &toml::Table) -> Result<Self> {
        let at = |k: &str| format!("mcp_servers.{name}.{k}");
        let string = |k: &str| -> Result<Option<String>> {
            match t.get(k) {
                None => Ok(None),
                Some(toml::Value::String(s)) => Ok(Some(s.clone())),
                Some(_) => bail!("{}: expected a string", at(k)),
            }
        };
        let strings = |k: &str| -> Result<Vec<String>> {
            match t.get(k) {
                None => Ok(Vec::new()),
                Some(toml::Value::Array(a)) => a
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .with_context(|| format!("{}: expected strings", at(k)))
                    })
                    .collect(),
                Some(_) => bail!("{}: expected an array", at(k)),
            }
        };
        let map = |k: &str| -> Result<BTreeMap<String, String>> {
            match t.get(k) {
                None => Ok(BTreeMap::new()),
                Some(toml::Value::Table(m)) => m
                    .iter()
                    .map(|(mk, v)| {
                        let v = v
                            .as_str()
                            .with_context(|| format!("{}.{mk}: expected a string", at(k)))?;
                        Ok((mk.clone(), v.to_string()))
                    })
                    .collect(),
                Some(_) => bail!("{}: expected a table", at(k)),
            }
        };
        let enabled = match t.get("enabled") {
            None => true,
            Some(toml::Value::Boolean(b)) => *b,
            Some(_) => bail!("{}: expected a boolean", at("enabled")),
        };
        const KNOWN: &[&str] = &[
            "command",
            "args",
            "env",
            "env_vars",
            "url",
            "http_headers",
            "env_http_headers",
            "bearer_token_env_var",
            "enabled",
        ];
        let mut env_vars = strings("env_vars")?;
        env_vars.sort();
        Ok(Self {
            command: string("command")?,
            args: strings("args")?,
            env: map("env")?,
            env_vars,
            url: string("url")?,
            http_headers: map("http_headers")?,
            env_http_headers: map("env_http_headers")?,
            bearer_token_env_var: string("bearer_token_env_var")?,
            enabled,
            other_keys: t
                .keys()
                .filter(|k| !KNOWN.contains(&k.as_str()))
                .cloned()
                .collect(),
        })
    }

    /// What onesrc would write for a source server, or why it cannot.
    fn render(s: &McpServer) -> Result<Self, String> {
        let mut out = CodexMcp {
            enabled: s.enabled,
            ..Default::default()
        };
        match &s.transport {
            Transport::Stdio { command, args, env } => {
                out.command = Some(command.clone());
                out.args = args.clone();
                for (k, v) in env {
                    match v {
                        Value::Literal(l) => {
                            out.env.insert(k.clone(), l.clone());
                        }
                        Value::EnvRef(var) if var == k => out.env_vars.push(var.clone()),
                        Value::EnvRef(var) => {
                            return Err(format!(
                                "env {k} = ${{{var}}}: Codex forwards variables by name only \
                                 (env_vars); use ${{{k}}}, or wait for write-time resolution (M3)"
                            ))
                        }
                    }
                }
                out.env_vars.sort();
            }
            Transport::Http {
                url,
                headers,
                bearer_token_env,
            } => {
                out.url = Some(url.clone());
                out.bearer_token_env_var = bearer_token_env.clone();
                for (h, v) in headers {
                    match v {
                        Value::Literal(l) => out.http_headers.insert(h.clone(), l.clone()),
                        Value::EnvRef(var) => out.env_http_headers.insert(h.clone(), var.clone()),
                    };
                }
            }
        }
        Ok(out)
    }

    /// Names of modelled fields that differ. Values are never printed.
    fn diff(&self, other: &Self) -> Vec<&'static str> {
        let mut d = Vec::new();
        macro_rules! cmp {
            ($($f:ident),*) => { $( if self.$f != other.$f { d.push(stringify!($f)); } )* };
        }
        cmp!(
            command,
            args,
            env,
            env_vars,
            url,
            http_headers,
            env_http_headers,
            bearer_token_env_var,
            enabled
        );
        d
    }
}

/// `Ok(None)`: no config file. `Err`: unreadable or unexpected shape.
fn read_mcp(p: &Paths) -> Result<Option<BTreeMap<String, CodexMcp>>> {
    let text = match std::fs::read_to_string(&p.config) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context("reading Codex config"),
    };
    let doc: toml::Table = toml::from_str(&text).context("parsing Codex config.toml")?;
    let mut out = BTreeMap::new();
    match doc.get("mcp_servers") {
        None => {}
        Some(toml::Value::Table(servers)) => {
            for (name, v) in servers {
                let t = v
                    .as_table()
                    .with_context(|| format!("mcp_servers.{name}: expected a table"))?;
                out.insert(name.clone(), CodexMcp::from_table(name, t)?);
            }
        }
        Some(_) => bail!("mcp_servers: expected a table"),
    }
    Ok(Some(out))
}

fn shadow(p: &Paths) -> Option<String> {
    let text = std::fs::read_to_string(&p.instructions_override).ok()?;
    (!text.trim().is_empty())
        .then(|| "AGENTS.override.md is non-empty and takes precedence".to_string())
}

pub fn probe(ctx: &Ctx) -> ProbeReport {
    let p = paths(ctx);
    let executable = ctx.which("codex");
    let version = executable.as_ref().and_then(|exe| {
        let out = std::process::Command::new(exe)
            .arg("--version")
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    });

    let path = |role, path: &PathBuf| PathInfo {
        role,
        path: ctx.show(path),
        kind: PathKind::of(ctx, path),
    };
    let mut paths = vec![
        path("user_config", &p.config),
        path("instructions", &p.instructions),
        path("instructions_override", &p.instructions_override),
    ];
    paths.extend(p.skill_roots.iter().map(|r| path("skill_root", r)));

    use CheckResult::*;
    use Evidence::*;
    let parse = match read_mcp(&p) {
        Ok(Some(m)) => Check::new(
            "config.parse",
            Supported,
            Some(Observed),
            format!("{} MCP server(s)", m.len()),
        ),
        Ok(None) => Check::new("config.parse", Unknown, None, "no config file"),
        Err(e) => Check::new("config.parse", Error, Some(Observed), format!("{e:#}")),
    };
    let codex_home_root = match p.skill_roots[1].is_dir() {
        true => "directory exists; whether Codex discovers skills here is not verified by probe",
        false => "directory absent",
    };
    let checks = vec![
        parse,
        Check::new(
            "instructions.global_file",
            Supported,
            Some(Documented),
            "AGENTS.md in CODEX_HOME",
        ),
        Check::new(
            "instructions.override",
            Supported,
            Some(Documented),
            "non-empty AGENTS.override.md wins",
        ),
        Check::new(
            "skills.root.agents",
            Supported,
            Some(Documented),
            "~/.agents/skills",
        ),
        Check::new("skills.root.codex_home", Unknown, None, codex_home_root),
        Check::new(
            "skills.symlink_discovery",
            Supported,
            Some(Documented),
            "symlinked skill directories",
        ),
        Check::new(
            "mcp.env_vars",
            Supported,
            Some(Documented),
            "forward variables by name",
        ),
        Check::new(
            "mcp.bearer_token_env_var",
            Supported,
            Some(Documented),
            "bearer token from a variable",
        ),
        Check::new(
            "mcp.env_http_headers",
            Supported,
            Some(Documented),
            "header values from variables",
        ),
        Check::new(
            "mcp.dollar_brace_expansion",
            Unsupported,
            Some(Documented),
            "no general ${VAR} expansion",
        ),
        Check::new(
            "config.runtime_rewrite",
            Unknown,
            None,
            "behavioural probe not run (M2)",
        ),
    ];

    ProbeReport {
        schema_version: SCHEMA_VERSION,
        agent: AgentInfo {
            id: AGENT,
            executable: executable.map(|e| ctx.show(&e)),
            version,
        },
        platform: Platform::current(),
        paths,
        checks,
        redacted: true,
    }
}

pub fn inspect(ctx: &Ctx) -> Result<String> {
    let p = paths(ctx);
    let mut out = format!("# codex: {}\n", ctx.show(&p.config));
    match read_mcp(&p)? {
        None => out.push_str("# (no config file)\n"),
        Some(servers) => {
            for (name, m) in &servers {
                out.push_str(&inspect_server(ctx, name, m));
            }
        }
    }
    for (role, file) in [
        ("instructions", &p.instructions),
        ("instructions override", &p.instructions_override),
    ] {
        if let Ok(text) = std::fs::read_to_string(file) {
            out.push_str(&format!(
                "# {role}: {} ({} lines)\n",
                ctx.show(file),
                text.lines().count()
            ));
        }
    }
    for root in &p.skill_roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        let mut names: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join("SKILL.md").is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .collect();
        names.sort();
        if !names.is_empty() {
            out.push_str(&format!(
                "# skills in {}: {}\n",
                ctx.show(root),
                names.join(", ")
            ));
        }
    }
    Ok(out)
}

fn inspect_server(ctx: &Ctx, name: &str, m: &CodexMcp) -> String {
    let q = |s: &str| toml::Value::String(s.to_string()).to_string();
    let key = |s: &str| match s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        true => s.to_string(),
        false => q(s),
    };
    let mut out = format!("\n[mcp.{}]\n", key(name));
    if let Some(c) = &m.command {
        out.push_str(&format!(
            "command = {}\n",
            q(&ctx.to_source_syntax(&scrub(c)))
        ));
    }
    if !m.args.is_empty() {
        let args: Vec<String> = m
            .args
            .iter()
            .map(|a| q(&ctx.to_source_syntax(&scrub(a))))
            .collect();
        out.push_str(&format!("args = [{}]\n", args.join(", ")));
    }
    let mut env: Vec<String> = m
        .env
        .keys()
        .map(|k| format!("{} = {}", key(k), q(MASK)))
        .collect();
    env.extend(
        m.env_vars
            .iter()
            .map(|v| format!("{} = {}", key(v), q(&format!("${{{v}}}")))),
    );
    if !env.is_empty() {
        out.push_str(&format!("env = {{ {} }}\n", env.join(", ")));
    }
    if let Some(u) = &m.url {
        out.push_str(&format!("url = {}\n", q(&scrub(u))));
    }
    let mut headers: Vec<String> = m
        .http_headers
        .keys()
        .map(|h| format!("{} = {}", q(h), q(MASK)))
        .collect();
    headers.extend(
        m.env_http_headers
            .iter()
            .map(|(h, v)| format!("{} = {}", q(h), q(&format!("${{{v}}}")))),
    );
    if !headers.is_empty() {
        out.push_str(&format!("headers = {{ {} }}\n", headers.join(", ")));
    }
    if let Some(v) = &m.bearer_token_env_var {
        out.push_str(&format!("bearer_token = {}\n", q(&format!("${{{v}}}"))));
    }
    if !m.enabled {
        out.push_str("enabled = false\n");
    }
    if !m.env.is_empty() || !m.http_headers.is_empty() {
        out.push_str(
            "# *** = literal value redacted; move it to secrets.env and reference it as ${VAR}\n",
        );
    }
    if !m.other_keys.is_empty() {
        out.push_str(&format!(
            "# not modelled by onesrc yet: {}\n",
            m.other_keys.join(", ")
        ));
    }
    out
}

pub fn status(ctx: &Ctx, source: &Source) -> Result<Vec<StatusRecord>> {
    let p = paths(ctx);
    let mut out = Vec::new();

    if let Some(r) = common::instructions_status(ctx, AGENT, source, &p.instructions, shadow(&p))? {
        out.push(r);
    }
    for skill in &source.skills {
        out.push(common::skill_status(ctx, AGENT, skill, &p.skill_roots));
    }
    out.extend(common::unmanaged_skills(
        ctx,
        AGENT,
        &p.skill_roots,
        source,
    )?);

    let current = read_mcp(&p)?;
    let path = ctx.show(&p.config);
    let rec = |item: &str, state, detail: Option<String>| StatusRecord {
        agent: AGENT,
        kind: ItemKind::Mcp,
        item: item.to_string(),
        path: path.clone(),
        state,
        owned: false,
        detail,
    };
    let empty = BTreeMap::new();
    let current_ref = current.as_ref().unwrap_or(&empty);
    let mut desired_names = Vec::new();
    for s in source.mcp.iter().filter(|s| s.targets(AGENT)) {
        desired_names.push(s.name.as_str());
        let want = match CodexMcp::render(s) {
            Ok(w) => w,
            Err(why) => {
                out.push(rec(&s.name, State::Unknown, Some(why)));
                continue;
            }
        };
        let r = match current_ref.get(&s.name) {
            None => rec(
                &s.name,
                State::Missing,
                current.is_none().then(|| "no config file".into()),
            ),
            Some(have) => match have.diff(&want) {
                fields if fields.is_empty() => rec(
                    &s.name,
                    State::InSync,
                    (!have.other_keys.is_empty())
                        .then(|| format!("also has: {}", have.other_keys.join(", "))),
                ),
                fields => rec(
                    &s.name,
                    State::Conflict,
                    Some(format!(
                        "not owned by onesrc and differs in: {}",
                        fields.join(", ")
                    )),
                ),
            },
        };
        out.push(r);
    }
    for name in current_ref
        .keys()
        .filter(|n| !desired_names.contains(&n.as_str()))
    {
        out.push(rec(name, State::Unmanaged, None));
    }
    Ok(out)
}
