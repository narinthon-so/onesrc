mod common;

use common::{symlink, write, Fixture};

/// A Codex home with every state `status` can report.
fn codex_fixture() -> Fixture {
    let f = Fixture::new();
    let (h, s) = (&f.home, &f.source);

    write(
        &h.join(".codex/config.toml"),
        r#"# user comment that must survive (M1)
model = "some-model"

[projects."/synthetic/project"]
trust_level = "trusted"

[mcp_servers.local-tool]
command = "node"
args = ["/opt/tools/local-tool/index.js"]
env_vars = ["LOCAL_TOOL_KEY"]

[mcp_servers.remote]
url = "https://mcp.example.com/mcp"
bearer_token_env_var = "REMOTE_TOKEN"
startup_timeout_sec = 20

[mcp_servers.drifted]
command = "npx"
args = ["drifted-mcp", "--old-flag"]

[mcp_servers.handmade]
command = "handmade-mcp"
args = ["--api-key=sk-PLANTED0secret0value0123", "ghp_PLANTED0token0value0456"]
env = { SERVICE_TOKEN = "sk-PLANTED0secret0value0123" }

[mcp_servers.handmade-http]
url = "https://api.example.com/mcp?key=PLANTED0query0value0abc0123&mode=1"
http_headers = { "X-Api-Key" = "PLANTED0header0value0789xyz" }
enabled = false
"#,
    );

    write(
        &s.join("onesrc.toml"),
        r#"[mcp.local-tool]
command = "node"
args = ["/opt/tools/local-tool/index.js"]
env = { LOCAL_TOOL_KEY = "${LOCAL_TOOL_KEY}" }

[mcp.remote]
url = "https://mcp.example.com/mcp"
bearer_token = "${REMOTE_TOKEN}"

[mcp.drifted]
command = "npx"
args = ["drifted-mcp"]

[mcp.not-installed]
command = "new-mcp"

[mcp.renamed-env]
command = "renamed"
env = { API_KEY = "${OTHER_NAME}" }

[mcp.claude-only]
command = "only-claude"
targets = ["claude"]
"#,
    );

    // instructions: identical copy
    write(&s.join("AGENTS.md"), "# Shared rules\n\nBe brief.\n");
    write(&h.join(".codex/AGENTS.md"), "# Shared rules\n\nBe brief.\n");

    // skills: linked, copied, drifted copy, missing, dangling, unmanaged, non-skill dir
    for name in ["linked", "copied", "drifted", "absent"] {
        write(
            &s.join(format!("skills/{name}/SKILL.md")),
            &format!("---\nname: {name}\n---\n"),
        );
    }
    write(&s.join("skills/copied/scripts/run.sh"), "echo hi\n");
    let agents_skills = h.join(".agents/skills");
    symlink(&s.join("skills/linked"), &agents_skills.join("linked"));
    write(
        &agents_skills.join("copied/SKILL.md"),
        "---\nname: copied\n---\n",
    );
    write(&agents_skills.join("copied/scripts/run.sh"), "echo hi\n");
    write(
        &agents_skills.join("drifted/SKILL.md"),
        "---\nname: drifted\n---\nlocal edit\n",
    );
    symlink(&h.join("gone/elsewhere"), &agents_skills.join("broken"));
    write(
        &h.join(".codex/skills/extra/SKILL.md"),
        "---\nname: extra\n---\n",
    );
    write(
        &h.join(".codex/skills/eval-workspace/notes.txt"),
        "not a skill\n",
    );
    f
}

fn stable(f: &Fixture, text: &str) -> String {
    text.replace(&f.home.to_string_lossy().to_string(), "{home}")
}

#[test]
fn status_reports_every_state() {
    let f = codex_fixture();
    let (out, err, code) = f.run(&["status"]);
    assert_eq!(err, "");
    assert_eq!(code, 1, "drift must exit 1");
    insta::assert_snapshot!(stable(&f, &out));
}

#[test]
fn status_json_is_versioned() {
    let f = codex_fixture();
    let (out, _, code) = f.run(&["status", "--json"]);
    assert_eq!(code, 1);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["schema_version"], 1);
    assert!(v["records"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["owned"] == false));
}

#[test]
fn inspect_is_redacted() {
    let f = codex_fixture();
    let (out, err, code) = f.run(&["inspect"]);
    assert_eq!((err.as_str(), code), ("", 0));
    insta::assert_snapshot!(stable(&f, &out));
}

#[test]
fn probe_text() {
    let f = codex_fixture();
    let (out, _, code) = f.run(&["probe"]);
    assert_eq!(code, 0);
    insta::assert_snapshot!(stable(&f, &out));
}

#[test]
fn probe_json_contract() {
    let f = codex_fixture();
    let (out, _, _) = f.run(&["probe", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let r = &v[0];
    assert_eq!(r["schema_version"], 1);
    assert_eq!(r["redacted"], true);
    assert_eq!(r["agent"]["id"], "codex");
    for c in r["checks"].as_array().unwrap() {
        let result = c["result"].as_str().unwrap();
        assert!(["supported", "unsupported", "unknown", "error"].contains(&result));
        // no evidence means unknown, never unsupported
        if c["evidence"].is_null() {
            assert_eq!(result, "unknown", "{c}");
        }
    }
}

#[test]
fn clean_when_in_sync() {
    let f = Fixture::new();
    write(
        &f.source.join("onesrc.toml"),
        "[mcp.one]\ncommand = \"one\"\n",
    );
    write(
        &f.home.join(".codex/config.toml"),
        "[mcp_servers.one]\ncommand = \"one\"\n",
    );
    let (out, _, code) = f.run(&["status"]);
    assert_eq!(code, 0, "{out}");
}

#[test]
fn override_shadows_instructions() {
    let f = Fixture::new();
    write(&f.source.join("AGENTS.md"), "rules\n");
    write(&f.home.join(".codex/AGENTS.md"), "rules\n");
    write(&f.home.join(".codex/AGENTS.override.md"), "other rules\n");
    let (out, _, code) = f.run(&["status"]);
    assert_eq!(code, 1);
    assert!(out.contains("shadowed"), "{out}");
}

#[test]
fn managed_block_counts_as_in_sync() {
    let f = Fixture::new();
    write(&f.source.join("AGENTS.md"), "rules\n");
    write(
        &f.home.join(".codex/AGENTS.md"),
        "# my own notes\n<!-- onesrc:start -->\nrules\n<!-- onesrc:end -->\nmore notes\n",
    );
    let (out, _, code) = f.run(&["status"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("managed block"), "{out}");
}

#[test]
fn codex_home_env_is_respected() {
    let f = Fixture::new();
    write(
        &f.source.join("onesrc.toml"),
        "[mcp.one]\ncommand = \"one\"\n",
    );
    write(
        &f.home.join("alt/config.toml"),
        "[mcp_servers.one]\ncommand = \"one\"\n",
    );
    let out = f
        .cmd(&["status"])
        .env("CODEX_HOME", f.home.join("alt"))
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn tilde_in_source_is_an_error() {
    let f = Fixture::new();
    write(
        &f.source.join("onesrc.toml"),
        "[mcp.x]\ncommand = \"node\"\nargs = [\"~/x.js\"]\n",
    );
    let (_, err, code) = f.run(&["status"]);
    assert_eq!(code, 2);
    assert!(err.contains("use {home}"), "{err}");
}

#[test]
fn unexpected_shape_refused() {
    let f = Fixture::new();
    write(&f.source.join("onesrc.toml"), "");
    write(
        &f.home.join(".codex/config.toml"),
        "[mcp_servers.x]\nargs = \"not-an-array\"\n",
    );
    let (_, err, code) = f.run(&["status"]);
    assert_eq!(code, 2);
    assert!(err.contains("mcp_servers.x.args"), "{err}");
}

#[test]
fn unimplemented_agent_is_an_error() {
    let f = Fixture::new();
    let (_, err, code) = f.run(&["probe", "--agent", "agy"]);
    assert_eq!(code, 2);
    assert!(err.contains("not implemented"), "{err}");
}

#[test]
fn inspect_rewrites_home_paths() {
    let f = Fixture::new();
    let script = f.home.join("tools/x/index.js");
    write(
        &f.home.join(".codex/config.toml"),
        &format!(
            "[mcp_servers.x]\ncommand = \"node\"\nargs = [{:?}]\n",
            script.to_string_lossy()
        ),
    );
    let (out, _, _) = f.run(&["inspect"]);
    assert!(
        out.contains(r#"args = ["{home}/tools/x/index.js"]"#),
        "{out}"
    );
}
