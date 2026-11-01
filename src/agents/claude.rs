//! Claude Code adapter (read-only part, M0).

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{bail, Context as _, Result};

use super::common;
use crate::ctx::Ctx;
use crate::redact::{scrub, scrub_args, MASK};
use crate::report::{
    AgentInfo, Check, CheckResult, Evidence, ItemKind, PathInfo, PathKind, Platform, ProbeReport,
    State, StatusRecord, SCHEMA_VERSION,
};
use crate::source::{AgentId, McpServer, Source, Transport, Value};

const AGENT: AgentId = AgentId::Claude;

struct Paths {
    config: PathBuf,
    instructions: PathBuf,
    skill_roots: Vec<PathBuf>,
}

fn paths(ctx: &Ctx) -> Paths {
    let config_dir = ctx
        .env("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| ctx.home.join(".claude"));
    Paths {
        config: ctx.home.join(".claude.json"),
        instructions: config_dir.join("CLAUDE.md"),
        skill_roots: vec![config_dir.join("skills")],
    }
}

/// One `mcpServers.<name>` object in Claude's format.
#[derive(Debug, Default, PartialEq, Eq)]
struct ClaudeMcp {
    r#type: Option<String>,
    command: Option<String>,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    url: Option<String>,
    headers: BTreeMap<String, String>,
    /// Keys onesrc does not model. Not compared.
    other_keys: Vec<String>,
}

impl ClaudeMcp {
    fn from_object(name: &str, obj: &serde_json::Map<String, serde_json::Value>) -> Result<Self> {
        let at = |k: &str| format!("mcpServers.{name}.{k}");
        let string = |k: &str| -> Result<Option<String>> {
            match obj.get(k) {
                None => Ok(None),
                Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
                Some(_) => bail!("{}: expected a string", at(k)),
            }
        };
        let strings = |k: &str| -> Result<Vec<String>> {
            match obj.get(k) {
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
        let map = |k: &str| -> Result<BTreeMap<String, String>> {
            match obj.get(k) {
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
        const KNOWN: &[&str] = &["type", "command", "args", "env", "url", "headers"];
        let mut other_keys: Vec<String> = obj
            .keys()
            .filter(|k| !KNOWN.contains(&k.as_str()))
            .cloned()
            .collect();
        other_keys.sort();

        Ok(Self {
            r#type: string("type")?,
            command: string("command")?,
            args: strings("args")?,
            env: map("env")?,
            url: string("url")?,
            headers: map("headers")?,
            other_keys,
        })
    }

    /// What onesrc would write for a source server, or why it cannot.
    fn render(s: &McpServer) -> Result<Self, String> {
        if !s.enabled {
            return Err(
                "enabled = false: Claude has no confirmed disable field; wait for M3".to_string(),
            );
        }
        let mut out = ClaudeMcp::default();
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
                                "env {k} = ${{{var}}}: Claude env var expansion is unverified; \
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
                out.r#type = Some("http".to_string());
                out.url = Some(url.clone());
                if let Some(var) = bearer_token_env {
                    return Err(format!(
                        "bearer_token = ${{{var}}}: bearer token support is unverified in M0; \
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
                                "headers {h} = ${{{var}}}: header variable expansion is unverified in M0; \
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
        if !type_matches(self.r#type.as_deref(), other.r#type.as_deref()) {
            d.push("type");
        }
        if self.command != other.command {
            d.push("command");
        }
        if self.args != other.args {
            d.push("args");
        }
        if self.env != other.env {
            d.push("env");
        }
        if self.url != other.url {
            d.push("url");
        }
        if self.headers != other.headers {
            d.push("headers");
        }
        d
    }
}

fn type_matches(a: Option<&str>, b: Option<&str>) -> bool {
    fn norm(t: Option<&str>) -> &str {
        match t {
            None | Some("stdio") => "stdio",
            Some(other) => other,
        }
    }
    norm(a) == norm(b)
}

struct ValueNoDuplicates;

impl<'de> serde::de::Visitor<'de> for ValueNoDuplicates {
    type Value = serde_json::Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("any valid JSON value")
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

    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(v)
            .map(serde_json::Value::Number)
            .ok_or_else(|| serde::de::Error::custom("invalid float"))
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
        Ok(serde_json::Value::String(v.to_owned()))
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
        deserializer.deserialize_any(self)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Null)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let mut vec = Vec::new();
        while let Some(elem) = seq.next_element_seed(ValueNoDuplicatesSeed)? {
            vec.push(elem);
        }
        Ok(serde_json::Value::Array(vec))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let mut m = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            let value = map.next_value_seed(ValueNoDuplicatesSeed)?;
            if m.insert(key.clone(), value).is_some() {
                return Err(serde::de::Error::custom(format!(
                    "duplicate key {key:?} in JSON"
                )));
            }
        }
        Ok(serde_json::Value::Object(m))
    }
}

struct ValueNoDuplicatesSeed;

impl<'de> serde::de::DeserializeSeed<'de> for ValueNoDuplicatesSeed {
    type Value = serde_json::Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(ValueNoDuplicates)
    }
}

fn parse_json_no_duplicates(text: &str) -> Result<serde_json::Value> {
    use serde::de::DeserializeSeed;
    let mut de = serde_json::Deserializer::from_str(text);
    let val = ValueNoDuplicatesSeed.deserialize(&mut de)?;
    de.end()?;
    Ok(val)
}

/// `Ok(None)`: no config file. `Err`: unreadable, duplicate keys, or unexpected shape.
/// Reads only `mcpServers`, completely ignoring any account or session state.
fn read_mcp(p: &Paths) -> Result<Option<BTreeMap<String, ClaudeMcp>>> {
    let text = match std::fs::read_to_string(&p.config) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context("reading Claude config"),
    };
    let doc: serde_json::Value =
        parse_json_no_duplicates(&text).context("parsing Claude .claude.json")?;
    let doc_obj = doc
        .as_object()
        .context("parsing Claude .claude.json: expected a top-level JSON object")?;
    let mut out = BTreeMap::new();
    match doc_obj.get("mcpServers") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::Object(servers)) => {
            for (name, v) in servers {
                let obj = v
                    .as_object()
                    .with_context(|| format!("mcpServers.{name}: expected an object"))?;
                out.insert(name.clone(), ClaudeMcp::from_object(name, obj)?);
            }
        }
        Some(_) => bail!("mcpServers: expected an object"),
    }
    Ok(Some(out))
}

pub fn probe(ctx: &Ctx) -> ProbeReport {
    let p = paths(ctx);
    let executable = ctx.which("claude");
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
    let user_config_info = if ctx.env("CLAUDE_CONFIG_DIR").is_some() {
        PathInfo {
            role: "user_config",
            path: "(unconfirmed)".to_string(),
            kind: PathKind::Other,
        }
    } else {
        path("user_config", &p.config)
    };
    let mut paths = vec![user_config_info, path("instructions", &p.instructions)];
    paths.extend(p.skill_roots.iter().map(|r| path("skill_root", r)));

    use CheckResult::*;
    use Evidence::*;
    let parse = if ctx.env("CLAUDE_CONFIG_DIR").is_some() {
        Check::new(
            "config.parse",
            Unknown,
            None,
            "where .claude.json lives when CLAUDE_CONFIG_DIR is set is not confirmed",
        )
    } else {
        match read_mcp(&p) {
            Ok(Some(m)) => Check::new(
                "config.parse",
                Supported,
                Some(Observed),
                format!("{} MCP server(s)", m.len()),
            ),
            Ok(None) => Check::new("config.parse", Unknown, None, "no config file"),
            Err(e) => Check::new("config.parse", Error, Some(Observed), format!("{e:#}")),
        }
    };
    let checks = vec![
        parse,
        Check::new(
            "config.claude_json_location",
            Unknown,
            None,
            "where .claude.json lives when CLAUDE_CONFIG_DIR is set is not confirmed",
        ),
        Check::new(
            "instructions.global_file",
            Supported,
            Some(Documented),
            "CLAUDE.md in config directory",
        ),
        Check::new(
            "instructions.imports",
            Supported,
            Some(Documented),
            "@path imports supported by Claude; compared as plain text in M0",
        ),
        Check::new(
            "skills.root",
            Supported,
            Some(Documented),
            "~/.claude/skills",
        ),
        Check::new(
            "skills.symlink_discovery",
            Supported,
            Some(Documented),
            "symlinked skill directories",
        ),
        Check::new(
            "mcp.types",
            Supported,
            Some(Documented),
            "stdio, http, sse transports",
        ),
        Check::new(
            "mcp.dollar_brace_expansion",
            Unknown,
            None,
            "whether Claude expands ${VAR} in .claude.json is unverified",
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
    let mut out = if ctx.env("CLAUDE_CONFIG_DIR").is_some() {
        "# claude: (unconfirmed when CLAUDE_CONFIG_DIR is set)\n# (no confirmed config file)\n"
            .to_string()
    } else {
        let mut out = format!("# claude: {}\n", ctx.show(&p.config));
        match read_mcp(&p)? {
            None => out.push_str("# (no config file)\n"),
            Some(servers) => {
                for (name, m) in &servers {
                    out.push_str(&inspect_server(ctx, name, m));
                }
            }
        }
        out
    };
    if let Ok(text) = std::fs::read_to_string(&p.instructions) {
        out.push_str(&format!(
            "# instructions: {} ({} lines)\n",
            ctx.show(&p.instructions),
            text.lines().count()
        ));
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

fn inspect_server(ctx: &Ctx, name: &str, m: &ClaudeMcp) -> String {
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
    if let Some(u) = &m.url {
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
    if m.r#type.as_deref() == Some("sse") {
        out.push_str("# not modelled: type = \"sse\"\n");
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

    if let Some(r) = common::instructions_status(ctx, AGENT, source, &p.instructions, None)? {
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

    if ctx.env("CLAUDE_CONFIG_DIR").is_some() {
        for s in source.mcp.iter().filter(|s| s.targets(AGENT)) {
            out.push(StatusRecord {
                agent: AGENT,
                kind: ItemKind::Mcp,
                item: s.name.clone(),
                path: "(unconfirmed)".to_string(),
                state: State::Unknown,
                owned: false,
                detail: Some(
                    "location of .claude.json when CLAUDE_CONFIG_DIR is set is unconfirmed (M0)"
                        .to_string(),
                ),
            });
        }
        return Ok(out);
    }

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
        let want = match ClaudeMcp::render(s) {
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
