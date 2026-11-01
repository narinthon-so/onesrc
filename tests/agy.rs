mod common;

use common::{symlink, write, Fixture};

/// An agy home with every state `status` can report.
fn agy_fixture() -> Fixture {
    let f = Fixture::new();
    let (h, s) = (&f.home, &f.source);

    write(
        &h.join(".gemini/config/mcp_config.json"),
        r#"{
  "mcpServers": {
    "local-tool": {
      "command": "node",
      "args": ["/opt/tools/local-tool/index.js"],
      "disabled": false
    },
    "remote": {
      "serverUrl": "https://mcp.example.com/mcp",
      "disabled": false,
      "startupTimeoutSec": 20
    },
    "disabled-server": {
      "command": "echo",
      "args": ["off"],
      "disabled": true
    },
    "drifted": {
      "command": "npx",
      "args": ["drifted-mcp", "--old-flag"],
      "disabled": false
    },
    "handmade": {
      "command": "handmade-mcp",
      "args": ["--api-key=sk-PLANTED0secret0value0123", "ghp_PLANTED0token0value0456"],
      "env": {
        "SERVICE_TOKEN": "sk-PLANTED0secret0value0123"
      },
      "disabled": false
    },
    "handmade-http": {
      "serverUrl": "https://api.example.com/mcp?key=PLANTED0query0value0abc0123&mode=1",
      "headers": {
        "X-Api-Key": "PLANTED0header0value0789xyz"
      },
      "disabled": true
    }
  }
}"#,
    );

    // IDE file that must be ignored
    write(
        &h.join(".gemini/antigravity/mcp_config.json"),
        r#"{"mcpServers": {"ide-only": {"command": "ide-tool", "disabled": false}}}"#,
    );

    // Credentials / internal state that must never be read
    write(
        &h.join(".gemini/oauth_creds.json"),
        r#"{"oauth_token": "sk-PLANTED0secret0value0123"}"#,
    );
    write(
        &h.join(".gemini/antigravity-cli/state.json"),
        r#"{"secret": "ghp_PLANTED0token0value0456"}"#,
    );

    write(
        &s.join("onesrc.toml"),
        r#"[mcp.local-tool]
command = "node"
args = ["/opt/tools/local-tool/index.js"]

[mcp.remote]
url = "https://mcp.example.com/mcp"

[mcp.disabled-server]
command = "echo"
args = ["off"]
enabled = false

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

    // instructions: two instruction files active at once
    write(&s.join("AGENTS.md"), "# Shared rules\n\nBe brief.\n");
    write(
        &h.join(".gemini/GEMINI.md"),
        "# Shared rules\n\nBe brief.\n",
    );
    write(
        &h.join(".gemini/AGENTS.md"),
        "# Shared rules\n\nBe brief.\n",
    );

    // skills: linked, copied, drifted, missing, dangling, unmanaged, non-skill dir
    for name in ["linked", "copied", "drifted", "absent"] {
        write(
            &s.join(format!("skills/{name}/SKILL.md")),
            &format!("---\nname: {name}\n---\n"),
        );
    }
    write(&s.join("skills/copied/scripts/run.sh"), "echo hi\n");

    let agy_skills = h.join(".gemini/config/skills");
    symlink(&s.join("skills/linked"), &agy_skills.join("linked"));
    write(
        &agy_skills.join("copied/SKILL.md"),
        "---\nname: copied\n---\n",
    );
    write(&agy_skills.join("copied/scripts/run.sh"), "echo hi\n");
    write(
        &agy_skills.join("drifted/SKILL.md"),
        "---\nname: drifted\n---\nlocal edit\n",
    );
    symlink(&h.join("gone/elsewhere"), &agy_skills.join("broken"));
    write(
        &h.join(".gemini/skills/extra/SKILL.md"),
        "---\nname: extra\n---\n",
    );
    write(
        &h.join(".gemini/skills/eval-workspace/notes.txt"),
        "not a skill\n",
    );
    f
}

fn stable(f: &Fixture, text: &str) -> String {
    text.replace(&f.home.to_string_lossy().to_string(), "{home}")
}

#[test]
fn status_reports_every_state() {
    let f = agy_fixture();
    let (out, err, code) = f.run(&["status", "--agent", "agy"]);
    assert_eq!(err, "");
    assert_eq!(code, 1, "drift must exit 1");
    assert!(!out.contains("ide-only"), "IDE server must be ignored");
    insta::assert_snapshot!(stable(&f, &out));
}

#[test]
fn status_json_is_versioned() {
    let f = agy_fixture();
    let (out, _, code) = f.run(&["status", "--json", "--agent", "agy"]);
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
    let f = agy_fixture();
    let (out, err, code) = f.run(&["inspect", "--agent", "agy"]);
    assert_eq!((err.as_str(), code), ("", 0));
    assert!(!out.contains("ide-only"), "IDE server must be ignored");
    insta::assert_snapshot!(stable(&f, &out));
}

#[test]
fn probe_text() {
    let f = agy_fixture();
    let (out, _, code) = f.run(&["probe", "--agent", "agy"]);
    assert_eq!(code, 0);
    assert!(!out.contains("ide-only"), "IDE server must be ignored");
    insta::assert_snapshot!(stable(&f, &out));
}

#[test]
fn probe_json_contract() {
    let f = agy_fixture();
    let (out, _, _) = f.run(&["probe", "--json", "--agent", "agy"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let r = &v[0];
    assert_eq!(r["schema_version"], 1);
    assert_eq!(r["redacted"], true);
    assert_eq!(r["agent"]["id"], "agy");
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
        &f.home.join(".gemini/config/mcp_config.json"),
        r#"{"mcpServers": {"one": {"command": "one", "disabled": false}}}"#,
    );
    let (out, _, code) = f.run(&["status", "--agent", "agy"]);
    assert_eq!(code, 0, "{out}");
}

#[test]
fn managed_block_counts_as_in_sync() {
    let f = Fixture::new();
    write(&f.source.join("AGENTS.md"), "rules\n");
    write(
        &f.home.join(".gemini/GEMINI.md"),
        "# my own notes\n<!-- onesrc:start -->\nrules\n<!-- onesrc:end -->\nmore notes\n",
    );
    let (out, _, code) = f.run(&["status", "--agent", "agy"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("managed block"), "{out}");
}

#[test]
fn ide_config_is_ignored() {
    let f = Fixture::new();
    write(&f.source.join("onesrc.toml"), "");
    write(
        &f.home.join(".gemini/antigravity/mcp_config.json"),
        r#"{"mcpServers": {"ide-tool": {"command": "ide", "disabled": false}}}"#,
    );
    let (out, _, _) = f.run(&["inspect", "--agent", "agy"]);
    assert!(!out.contains("ide-tool"), "IDE config must be ignored");
    assert!(out.contains("# (no config file)"), "{out}");
}

#[test]
fn unexpected_shape_refused() {
    let f = Fixture::new();
    write(&f.source.join("onesrc.toml"), "");
    write(
        &f.home.join(".gemini/config/mcp_config.json"),
        r#"{"mcpServers": {"x": {"args": "not-an-array"}}}"#,
    );
    let (_, err, code) = f.run(&["status", "--agent", "agy"]);
    assert_eq!(code, 2);
    assert!(err.contains("mcpServers.x.args"), "{err}");
}

#[test]
fn inspect_rewrites_home_paths() {
    let f = Fixture::new();
    let script = f.home.join("tools/x/index.js");
    write(
        &f.home.join(".gemini/config/mcp_config.json"),
        &format!(
            r#"{{"mcpServers": {{"x": {{"command": "node", "args": [{:?}]}}}}}}"#,
            script.to_string_lossy()
        ),
    );
    let (out, _, _) = f.run(&["inspect", "--agent", "agy"]);
    assert!(
        out.contains(r#"args = ["{home}/tools/x/index.js"]"#),
        "{out}"
    );
}
