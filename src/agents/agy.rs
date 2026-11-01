//! Google Antigravity CLI (`agy`) adapter (read-only part, M0).
//!
//! Facts marked "documented" come from the agy documentation or embedded
//! help strings; facts marked "observed" were confirmed by running agy.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use serde::de::Deserializer as _;

use super::common;
use crate::ctx::Ctx;
use crate::redact::{scrub, scrub_args, MASK};
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
    // Spec root is ~/.gemini/config/skills only.
    let skill_roots = vec![gemini_dir.join("config/skills")];
    Paths {
        gemini_dir,
        config,
        instruction_files,
        skill_roots,
    }
}

/// Check if a path resolves or links to a forbidden credential/internal location.
fn is_forbidden_target(ctx: &Ctx, path: &Path) -> bool {
    let target = if let Ok(real) = std::fs::canonicalize(path) {
        real
    } else if let Ok(link) = std::fs::read_link(path) {
        if link.is_relative() {
            path.parent().unwrap_or(path).join(&link)
        } else {
            link
        }
    } else {
        return false;
    };

    let gemini_dir = ctx.home.join(".gemini");
    let cli_dir = gemini_dir.join("antigravity-cli");
    if let Ok(real_cli) = std::fs::canonicalize(&cli_dir) {
        if target.starts_with(&real_cli) {
            return true;
        }
    } else if target.starts_with(&cli_dir) {
        return true;
    }

    for cred_name in [
        "oauth_creds.json",
        "google_accounts.json",
        "gemini-credentials.json",
        "settings.json",
        "antigravity/mcp_config.json",
    ] {
        let cred_path = gemini_dir.join(cred_name);
        if let Ok(real_cred) = std::fs::canonicalize(&cred_path) {
            if target == real_cred {
                return true;
            }
        } else if target == cred_path {
            return true;
        }
    }
    false
}

/// Walk a skill tree with symlink_metadata and refuse if any entry's canonical target
/// is a forbidden path or inside ~/.gemini/antigravity-cli/.
fn skill_tree_has_forbidden_link(ctx: &Ctx, root: &Path) -> bool {
    if is_forbidden_target(ctx, root) {
        return true;
    }
    let Ok(meta) = std::fs::symlink_metadata(root) else {
        return false;
    };
    if !meta.is_dir() && !root.is_dir() {
        return false;
    }

    let mut stack = vec![root.to_path_buf()];
    let mut visited = HashSet::new();
    if let Ok(real) = std::fs::canonicalize(root) {
        visited.insert(real);
    }

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if is_forbidden_target(ctx, &path) {
                return true;
            }
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() || (meta.is_symlink() && path.is_dir()) {
                if let Ok(real) = std::fs::canonicalize(&path) {
                    if visited.insert(real) {
                        stack.push(path);
                    }
                }
            }
        }
    }
    false
}

/// Serde visitor that validates there are no duplicate keys anywhere in JSON.
struct NoDuplicateKeys;

impl<'de> serde::de::Visitor<'de> for NoDuplicateKeys {
    type Value = serde_json::Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("any valid JSON value without duplicate keys")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Number(v.into()))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Number(v.into()))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E> {
        Ok(serde_json::Number::from_f64(v)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null))
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
        Ok(serde_json::Value::String(v.to_string()))
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
        Ok(serde_json::Value::String(v))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicateKeys)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Null)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let mut vec = Vec::new();
        while let Some(elem) = seq.next_element_seed(NoDuplicateKeys)? {
            vec.push(elem);
        }
        Ok(serde_json::Value::Array(vec))
    }

    fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
    where
        M: serde::de::MapAccess<'de>,
    {
        let mut map = serde_json::Map::new();
        while let Some(key) = access.next_key::<String>()? {
            if map.contains_key(&key) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate key in JSON: {key:?}"
                )));
            }
            let value = access.next_value_seed(NoDuplicateKeys)?;
            map.insert(key, value);
        }
        Ok(serde_json::Value::Object(map))
    }
}

impl<'de> serde::de::DeserializeSeed<'de> for NoDuplicateKeys {
    type Value = serde_json::Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicateKeys)
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
        let at = |k: &str| {
            if k.is_empty() {
                format!("mcpServers.{name}")
            } else {
                format!("mcpServers.{name}.{k}")
            }
        };
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

        let command = string("command")?;
        let server_url = string("serverUrl")?;
        if command.is_none() && server_url.is_none() {
            bail!("{}: expected command or serverUrl", at(""));
        }
        if command.is_some() && server_url.is_some() {
            bail!("{}: set command or serverUrl, not both", at(""));
        }

        const KNOWN: &[&str] = &["command", "args", "env", "serverUrl", "headers", "disabled"];
        let mut other_keys: Vec<String> = map
            .keys()
            .filter(|k| !KNOWN.contains(&k.as_str()))
            .cloned()
            .collect();
        other_keys.sort();

        Ok(Self {
            command,
            args: strings("args")?,
            env: string_map("env")?,
            server_url,
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
fn read_mcp(ctx: &Ctx, p: &Paths) -> Result<Option<BTreeMap<String, AgyMcp>>> {
    if is_forbidden_target(ctx, &p.config) {
        bail!(
            "refusing to read config symlinked to forbidden path: {}",
            ctx.show(&p.config)
        );
    }
    let text = match std::fs::read_to_string(&p.config) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context("reading agy config"),
    };
    let mut de = serde_json::Deserializer::from_str(&text);
    let doc = de
        .deserialize_any(NoDuplicateKeys)
        .context("parsing agy mcp_config.json")?;
    de.end().context("parsing agy mcp_config.json")?;
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
    let parse = match read_mcp(ctx, &p) {
        Ok(Some(m)) => Check::new(
            "config.parse",
            Supported,
            Some(Observed),
            format!("{} MCP server(s)", m.len()),
        ),
        Ok(None) => Check::new("config.parse", Unknown, None, "no config file"),
        Err(e) => Check::new("config.parse", Error, Some(Observed), format!("{e:#}")),
    };

    let gemini_skills = p.gemini_dir.join("skills");
    let gemini_skills_detail = match gemini_skills.is_dir() {
        true => "directory exists; seen in prior art but not used by CLI",
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
        Check::new("skills.root.gemini", Unknown, None, gemini_skills_detail),
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
            Unknown,
            None,
            "key name in mcpServers unconfirmed (agy accepts --env)",
        ),
        Check::new(
            "mcp.headers",
            Unknown,
            None,
            "key name in mcpServers unconfirmed (agy accepts --header)",
        ),
        Check::new(
            "mcp.dollar_brace_expansion",
            Unknown,
            None,
            "unconfirmed whether agy expands ${VAR} in values",
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
    match read_mcp(ctx, &p)? {
        None => out.push_str("# (no config file)\n"),
        Some(servers) => {
            for (name, m) in &servers {
                out.push_str(&inspect_server(ctx, name, m));
            }
        }
    }
    for file in &p.instruction_files {
        if is_forbidden_target(ctx, file) {
            out.push_str(&format!(
                "# instructions: {} (symlink lands in forbidden path; not read)\n",
                ctx.show(file)
            ));
            continue;
        }
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
        let rewritten = ctx.to_source_syntax(c);
        out.push_str(&format!("command = {}\n", q(&scrub(&rewritten))));
    }
    if !m.args.is_empty() {
        let rewritten: Vec<String> = m.args.iter().map(|a| ctx.to_source_syntax(a)).collect();
        let args: Vec<String> = scrub_args(&rewritten).iter().map(|a| q(a)).collect();
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
        let rewritten = ctx.to_source_syntax(u);
        out.push_str(&format!("url = {}\n", q(&scrub(&rewritten))));
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
        if is_forbidden_target(ctx, primary) {
            out.push(StatusRecord {
                agent: AGENT,
                kind: ItemKind::Instructions,
                item: "instructions".to_string(),
                path: ctx.show(primary),
                state: State::Unknown,
                owned: false,
                detail: Some("symlink lands in forbidden path; not read".to_string()),
            });
        } else if let Some(r) = common::instructions_status(ctx, AGENT, source, primary, None)? {
            out.push(r);
        }
        for extra in &existing_instructions[1..] {
            let detail = if is_forbidden_target(ctx, extra) {
                "symlink lands in forbidden path; not read".to_string()
            } else {
                "precedence unconfirmed".to_string()
            };
            out.push(StatusRecord {
                agent: AGENT,
                kind: ItemKind::Instructions,
                item: "instructions".to_string(),
                path: ctx.show(extra),
                state: State::Unknown,
                owned: false,
                detail: Some(detail),
            });
        }
    }

    for skill in &source.skills {
        if let Some(first) = p
            .skill_roots
            .iter()
            .map(|r| r.join(&skill.name))
            .find(|p| PathKind::of(ctx, p) != PathKind::Missing)
        {
            if skill_tree_has_forbidden_link(ctx, &first) {
                out.push(StatusRecord {
                    agent: AGENT,
                    kind: ItemKind::Skill,
                    item: skill.name.clone(),
                    path: ctx.show(&first),
                    state: State::Unknown,
                    owned: false,
                    detail: Some("skill links into a forbidden location".to_string()),
                });
                continue;
            }
        }
        out.push(common::skill_status(ctx, AGENT, skill, &p.skill_roots));
    }
    for mut rec in common::unmanaged_skills(ctx, AGENT, &p.skill_roots, source)? {
        let skill_path = p.skill_roots[0].join(&rec.item);
        if skill_tree_has_forbidden_link(ctx, &skill_path) {
            rec.state = State::Unknown;
            rec.detail = Some("skill links into a forbidden location".to_string());
        }
        out.push(rec);
    }

    let current = read_mcp(ctx, &p)?;
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
