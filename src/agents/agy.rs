//! Google Antigravity CLI (`agy`) adapter (read-only part, M0).
//!
//! Facts marked "documented" come from the agy documentation or embedded
//! help strings; facts marked "observed" were confirmed by running agy.

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

const AGENT: AgentId = AgentId::Agy;

struct Paths {
    gemini_dir: PathBuf,
    config: PathBuf,
    instruction_files: Vec<PathBuf>,
    /// Install root first (documented), then other roots seen in practice.
    skill_roots: Vec<PathBuf>,
}

fn paths(ctx: &Ctx) -> Paths {
    let gemini_dir = ctx.home.join(".gemini");
    let config = gemini_dir.join("config/mcp_config.json");
    let instruction_files = vec![
        gemini_dir.join("GEMINI.md"),
        gemini_dir.join("AGENTS.md"),
        gemini_dir.join("config/GEMINI.md"),
        gemini_dir.join("config/AGENTS.md"),
    ];
    let skill_roots = vec![gemini_dir.join("config/skills"), gemini_dir.join("skills")];
    Paths {
        gemini_dir,
        config,
        instruction_files,
        skill_roots,
    }
}

/// One MCP server entry from `mcp_config.json`'s `mcpServers` map.
#[derive(Debug, Default, PartialEq, Eq)]
struct AgyMcp {
    command: Option<String>,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    server_url: Option<String>,
    headers: BTreeMap<String, String>,
    disabled: bool,
    /// Keys onesrc does not model (e.g. timeouts). Not compared.
    other_keys: Vec<String>,
}

impl AgyMcp {
    fn from_json(name: &str, map: &serde_json::Map<String, serde_json::Value>) -> Result<Self> {
        let at = |k: &str| format!("mcpServers.{name}.{k}");
        let string = |k: &str| -> Result<Option<String>> {
            match map.get(k) {
                None => Ok(None),
                Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
                Some(_) => bail!("{}: expected a string", at(k)),
            }
        };
        let strings = |k: &str| -> Result<Vec<String>> {
            match map.get(k) {
                None => Ok(Vec::new()),
                Some(serde_json::Value::Array(a)) => a
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
        let string_map = |k: &str| -> Result<BTreeMap<String, String>> {
            match map.get(k) {
                None => Ok(BTreeMap::new()),
                Some(serde_json::Value::Object(m)) => m
                    .iter()
                    .map(|(mk, v)| {
                        let v = v
                            .as_str()
                            .with_context(|| format!("{}.{mk}: expected a string", at(k)))?;
                        Ok((mk.clone(), v.to_string()))
                    })
                    .collect(),
                Some(_) => bail!("{}: expected an object", at(k)),
            }
        };
        let disabled = match map.get("disabled") {
            None => false,
            Some(serde_json::Value::Bool(b)) => *b,
            Some(_) => bail!("{}: expected a boolean", at("disabled")),
        };
        const KNOWN: &[&str] = &["command", "args", "env", "serverUrl", "headers", "disabled"];
        let mut other_keys: Vec<String> = map
            .keys()
            .filter(|k| !KNOWN.contains(&k.as_str()))
            .cloned()
            .collect();
        other_keys.sort();

        Ok(Self {
            command: string("command")?,
            args: strings("args")?,
            env: string_map("env")?,
            server_url: string("serverUrl")?,
            headers: string_map("headers")?,
            disabled,
            other_keys,
        })
    }

    /// What onesrc would write for a source server, or why it cannot.
    fn render(s: &McpServer) -> Result<Self, String> {
        let mut out = AgyMcp {
            disabled: !s.enabled,
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
                        Value::EnvRef(var) => {
                            return Err(format!(
                                "env {k} = ${{{var}}}: agy does not expand ${{VAR}}; \
                                 wait for write-time resolution (M3)"
                            ));
                        }
                    }
                }
            }
            Transport::Http {
                url,
                headers,
                bearer_token_env,
            } => {
                out.server_url = Some(url.clone());
                if let Some(var) = bearer_token_env {
                    return Err(format!(
                        "bearer_token = ${{{var}}}: agy has no bearer token field; \
                         wait for write-time resolution (M3)"
                    ));
                }
                for (h, v) in headers {
                    match v {
                        Value::Literal(l) => {
                            out.headers.insert(h.clone(), l.clone());
                        }
                        Value::EnvRef(var) => {
                            return Err(format!(
                                "headers {h} = ${{{var}}}: agy does not expand ${{VAR}}; \
                                 wait for write-time resolution (M3)"
                            ));
                        }
                    }
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
        cmp!(command, args, env, server_url, headers, disabled);
        d
    }
}

/// `Ok(None)`: no config file. `Err`: unreadable or unexpected shape.
fn read_mcp(p: &Paths) -> Result<Option<BTreeMap<String, AgyMcp>>> {
    let text = match std::fs::read_to_string(&p.config) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context("reading agy config"),
    };
    let doc: serde_json::Value =
        serde_json::from_str(&text).context("parsing agy mcp_config.json")?;
    let doc_obj = doc
        .as_object()
        .context("parsing agy mcp_config.json: expected a top-level object")?;
    let mut out = BTreeMap::new();
    match doc_obj.get("mcpServers") {
        None => {}
        Some(serde_json::Value::Object(servers)) => {
            for (name, v) in servers {
                let obj = v
                    .as_object()
                    .with_context(|| format!("mcpServers.{name}: expected an object"))?;
                out.insert(name.clone(), AgyMcp::from_json(name, obj)?);
            }
        }
        Some(_) => bail!("mcpServers: expected an object"),
    }
    Ok(Some(out))
}

pub fn probe(ctx: &Ctx) -> ProbeReport {
    let p = paths(ctx);
    let executable = ctx.which("agy");
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
    let mut paths = vec![path("user_config", &p.config)];
    for inst in &p.instruction_files {
        paths.push(path("instructions", inst));
    }
    paths.extend(p.skill_roots.iter().map(|r| path("skill_root", r)));

    // Report existence only for credential / internal state files (never read content)
    for (role, file) in [
        ("oauth_creds", p.gemini_dir.join("oauth_creds.json")),
        ("google_accounts", p.gemini_dir.join("google_accounts.json")),
        (
            "gemini_credentials",
            p.gemini_dir.join("gemini-credentials.json"),
        ),
        ("internal_state", p.gemini_dir.join("antigravity-cli")),
    ] {
        if PathKind::of(ctx, &file) != PathKind::Missing {
            paths.push(path(role, &file));
        }
    }

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
    let gemini_skills_root = match p.skill_roots[1].is_dir() {
        true => "directory exists; whether agy discovers skills here is not verified by probe",
        false => "directory absent",
    };
    let existing_instructions: Vec<&PathBuf> = p
        .instruction_files
        .iter()
        .filter(|f| PathKind::of(ctx, f) != PathKind::Missing)
        .collect();
    let instructions_precedence = match existing_instructions.len() {
        0 => "no instruction files present; precedence unconfirmed",
        1 => "single instruction file present; precedence between candidate names unconfirmed",
        _ => "multiple instruction files present; precedence unconfirmed",
    };

    let checks = vec![
        parse,
        Check::new(
            "instructions.global_file",
            Supported,
            Some(Documented),
            "GEMINI.md or AGENTS.md in ~/.gemini or ~/.gemini/config",
        ),
        Check::new(
            "instructions.precedence",
            Unknown,
            None,
            instructions_precedence,
        ),
        Check::new(
            "skills.root.config",
            Supported,
            Some(Documented),
            "~/.gemini/config/skills",
        ),
        Check::new("skills.root.gemini", Unknown, None, gemini_skills_root),
        Check::new(
            "skills.symlink_discovery",
            Supported,
            Some(Documented),
            "symlinked skill directories",
        ),
        Check::new(
            "mcp.server_url",
            Supported,
            Some(Observed),
            "serverUrl field for remote MCP servers",
        ),
        Check::new(
            "mcp.disabled",
            Supported,
            Some(Observed),
            "disabled boolean field",
        ),
        Check::new(
            "mcp.env",
            Supported,
            Some(Documented),
            "env object in MCP server entry",
        ),
        Check::new(
            "mcp.headers",
            Supported,
            Some(Documented),
            "headers object in MCP server entry",
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
    let mut out = format!("# agy: {}\n", ctx.show(&p.config));
    match read_mcp(&p)? {
        None => out.push_str("# (no config file)\n"),
        Some(servers) => {
            for (name, m) in &servers {
                out.push_str(&inspect_server(ctx, name, m));
            }
        }
    }
    for file in &p.instruction_files {
        if let Ok(text) = std::fs::read_to_string(file) {
            out.push_str(&format!(
                "# instructions: {} ({} lines)\n",
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

fn inspect_server(ctx: &Ctx, name: &str, m: &AgyMcp) -> String {
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
    let env: Vec<String> = m
        .env
        .keys()
        .map(|k| format!("{} = {}", key(k), q(MASK)))
        .collect();
    if !env.is_empty() {
        out.push_str(&format!("env = {{ {} }}\n", env.join(", ")));
    }
    if let Some(u) = &m.server_url {
        out.push_str(&format!("url = {}\n", q(&scrub(u))));
    }
    let headers: Vec<String> = m
        .headers
        .keys()
        .map(|h| format!("{} = {}", q(h), q(MASK)))
        .collect();
    if !headers.is_empty() {
        out.push_str(&format!("headers = {{ {} }}\n", headers.join(", ")));
    }
    if m.disabled {
        out.push_str("enabled = false\n");
    }
    if !m.env.is_empty() || !m.headers.is_empty() {
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

    let existing_instructions: Vec<&PathBuf> = p
        .instruction_files
        .iter()
        .filter(|f| PathKind::of(ctx, f) != PathKind::Missing)
        .collect();

    if existing_instructions.is_empty() {
        if let Some(r) =
            common::instructions_status(ctx, AGENT, source, &p.instruction_files[0], None)?
        {
            out.push(r);
        }
    } else {
        let primary = existing_instructions[0];
        if let Some(r) = common::instructions_status(ctx, AGENT, source, primary, None)? {
            out.push(r);
        }
        for extra in &existing_instructions[1..] {
            let reason = format!(
                "also present: {}; agy may read multiple instruction files, precedence unconfirmed",
                ctx.show(primary)
            );
            if let Some(r) = common::instructions_status(ctx, AGENT, source, extra, Some(reason))? {
                out.push(r);
            }
        }
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
        let want = match AgyMcp::render(s) {
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
