mod common;

use common::{symlink, write, Fixture, SECRETS};

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
      "args": [
        "--api-key=sk-PLANTED0secret0value0123",
        "ghp_PLANTED0token0value0456",
        "--token",
        "PLANTED0two0arg0pw0777",
        "--token=https://example.com/PLANTED0url0key0val0secret111",
        "postgres://user:pw@host/db"
      ],
      "env": {
        "SERVICE_TOKEN": "sk-PLANTED0secret0value0123"
      },
      "disabled": false
    },
    "handmade-http": {
      "serverUrl": "https://ghp_PLANTED0user0token0222:PLANTED0userinfo0pw0999@api.example.com/mcp/ghp_PLANTED0path0token0888?key=PLANTED0query0value0abc0123&mode=1",
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

    // Spec root is ~/.gemini/config/skills
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
        &agy_skills.join("extra/SKILL.md"),
        "---\nname: extra\n---\n",
    );
    write(
        &agy_skills.join("eval-workspace/notes.txt"),
        "not a skill\n",
    );

    // Skills outside ~/.gemini/config/skills must not be used by status
    write(
        &h.join(".gemini/skills/outside/SKILL.md"),
        "---\nname: outside\n---\n",
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
    assert!(
        !out.contains("outside"),
        "skills outside config/skills must not be in status: {out}"
    );

    // Per-state assertions
    assert!(out.contains("in_sync"), "missing in_sync: {out}");
    assert!(out.contains("missing"), "missing missing: {out}");
    assert!(out.contains("conflict"), "missing conflict: {out}");
    assert!(out.contains("unmanaged"), "missing unmanaged: {out}");
    assert!(out.contains("unknown"), "missing unknown: {out}");
    assert!(
        out.contains("instructions unknown      instructions         {home}/.gemini/AGENTS.md\n                                  ↳ precedence unconfirmed"),
        "extra instruction file must be reported as unknown with note: {out}"
    );
    assert!(
        !out.contains("shadowed"),
        "instruction precedence unconfirmed should not be marked shadowed: {out}"
    );

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

    let states: Vec<&str> = v["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["state"].as_str().unwrap())
        .collect();
    assert!(states.contains(&"in_sync"));
    assert!(states.contains(&"missing"));
    assert!(states.contains(&"conflict"));
    assert!(states.contains(&"unmanaged"));
    assert!(states.contains(&"unknown"));
}

#[test]
fn inspect_is_redacted() {
    let f = agy_fixture();
    let (out, err, code) = f.run(&["inspect", "--agent", "agy"]);
    assert_eq!((err.as_str(), code), ("", 0));
    assert!(!out.contains("ide-only"), "IDE server must be ignored");
    assert!(
        !out.contains("outside"),
        "skills outside config/skills must not be inspected"
    );
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
fn managed_block_differs() {
    let f = Fixture::new();
    write(&f.source.join("AGENTS.md"), "rules from source\n");
    write(
        &f.home.join(".gemini/GEMINI.md"),
        "# user prefix\n<!-- onesrc:start -->\nold rules\n<!-- onesrc:end -->\nuser suffix\n",
    );
    let (out, _, code) = f.run(&["status", "--agent", "agy"]);
    assert_eq!(code, 1);
    assert!(out.contains("differs"), "{out}");
    assert!(out.contains("managed block differs from source"), "{out}");
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
        r#"{"mcpServers": {"x": {"command": "echo", "args": "not-an-array"}}}"#,
    );
    let (_, err, code) = f.run(&["status", "--agent", "agy"]);
    assert_eq!(code, 2);
    assert!(err.contains("mcpServers.x.args"), "{err}");
}

#[test]
fn entry_without_command_or_server_url_refused() {
    let f = Fixture::new();
    write(&f.source.join("onesrc.toml"), "");
    write(
        &f.home.join(".gemini/config/mcp_config.json"),
        r#"{"mcpServers": {"empty-server": {"disabled": false}}}"#,
    );
    let (_, err, code) = f.run(&["status", "--agent", "agy"]);
    assert_eq!(code, 2);
    assert!(
        err.contains("mcpServers.empty-server: expected command or serverUrl"),
        "{err}"
    );
}

#[test]
fn duplicate_server_name_refused() {
    let f = Fixture::new();
    write(&f.source.join("onesrc.toml"), "");
    write(
        &f.home.join(".gemini/config/mcp_config.json"),
        r#"{
  "mcpServers": {
    "srv": {"command": "echo"},
    "srv": {"command": "echo2"}
  }
}"#,
    );
    let (_, err, code) = f.run(&["status", "--agent", "agy"]);
    assert_eq!(code, 2);
    assert!(err.contains("duplicate key"), "{err}");
}

#[test]
fn duplicate_field_refused() {
    let f = Fixture::new();
    write(&f.source.join("onesrc.toml"), "");
    write(
        &f.home.join(".gemini/config/mcp_config.json"),
        r#"{
  "mcpServers": {
    "srv": {
      "command": "echo",
      "command": "echo2"
    }
  }
}"#,
    );
    let (_, err, code) = f.run(&["status", "--agent", "agy"]);
    assert_eq!(code, 2);
    assert!(err.contains("duplicate key"), "{err}");
}

#[test]
fn trailing_content_in_mcp_config_refused() {
    let f = Fixture::new();
    write(&f.source.join("onesrc.toml"), "");
    write(
        &f.home.join(".gemini/config/mcp_config.json"),
        r#"{"mcpServers": {}} garbage"#,
    );
    let (_, err, code) = f.run(&["status", "--agent", "agy"]);
    assert_eq!(code, 2);
    assert!(err.contains("trailing"), "{err}");

    let f2 = Fixture::new();
    write(&f2.source.join("onesrc.toml"), "");
    write(
        &f2.home.join(".gemini/config/mcp_config.json"),
        r#"{"mcpServers": {}} {"mcpServers": {}}"#,
    );
    let (_, err2, code2) = f2.run(&["status", "--agent", "agy"]);
    assert_eq!(code2, 2);
    assert!(err2.contains("trailing"), "{err2}");
}

#[test]
fn config_symlink_to_credential_refused() {
    let f = Fixture::new();
    write(&f.source.join("onesrc.toml"), "");
    write(
        &f.home.join(".gemini/oauth_creds.json"),
        r#"{"oauth_token": "sk-PLANTED0secret0value0123"}"#,
    );
    symlink(
        &f.home.join(".gemini/oauth_creds.json"),
        &f.home.join(".gemini/config/mcp_config.json"),
    );
    let (_, err, code) = f.run(&["status", "--agent", "agy"]);
    assert_eq!(code, 2);
    assert!(err.contains("forbidden path"), "{err}");
}

#[test]
fn config_symlink_to_settings_json_refused() {
    let f = Fixture::new();
    write(&f.source.join("onesrc.toml"), "");
    write(
        &f.home.join(".gemini/settings.json"),
        r#"{"mcpServers": {"settings-tool": {"command": "st"}}}"#,
    );
    symlink(
        &f.home.join(".gemini/settings.json"),
        &f.home.join(".gemini/config/mcp_config.json"),
    );
    let (out, err, code) = f.run(&["inspect", "--agent", "agy"]);
    assert_eq!(code, 2);
    assert!(err.contains("forbidden path"), "{err}");
    assert!(!out.contains("settings-tool"), "{out}");
}

#[test]
fn config_symlink_to_antigravity_mcp_config_refused() {
    let f = Fixture::new();
    write(&f.source.join("onesrc.toml"), "");
    write(
        &f.home.join(".gemini/antigravity/mcp_config.json"),
        r#"{"mcpServers": {"ide-tool": {"command": "ide"}}}"#,
    );
    symlink(
        &f.home.join(".gemini/antigravity/mcp_config.json"),
        &f.home.join(".gemini/config/mcp_config.json"),
    );
    let (out, err, code) = f.run(&["inspect", "--agent", "agy"]);
    assert_eq!(code, 2);
    assert!(err.contains("forbidden path"), "{err}");
    assert!(!out.contains("ide-tool"), "{out}");
}

#[test]
fn config_symlink_to_antigravity_cli_refused() {
    let f = Fixture::new();
    write(&f.source.join("onesrc.toml"), "");
    write(
        &f.home.join(".gemini/antigravity-cli/state.json"),
        r#"{"secret": "ghp_PLANTED0token0value0456"}"#,
    );
    symlink(
        &f.home.join(".gemini/antigravity-cli/state.json"),
        &f.home.join(".gemini/config/mcp_config.json"),
    );
    let (_, err, code) = f.run(&["status", "--agent", "agy"]);
    assert_eq!(code, 2);
    assert!(err.contains("forbidden path"), "{err}");
}

#[test]
fn instruction_symlink_to_credential_refused() {
    let f = Fixture::new();
    write(&f.source.join("AGENTS.md"), "rules\n");
    write(
        &f.home.join(".gemini/oauth_creds.json"),
        r#"{"oauth_token": "sk-PLANTED0secret0value0123"}"#,
    );
    symlink(
        &f.home.join(".gemini/oauth_creds.json"),
        &f.home.join(".gemini/GEMINI.md"),
    );
    let (out, _, _) = f.run(&["status", "--agent", "agy"]);
    assert!(out.contains("forbidden path"), "{out}");
    assert!(out.contains("unknown"), "{out}");
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

#[test]
fn inspect_rewrites_home_with_long_random_segment() {
    let f = Fixture::new();
    // Simulate macOS-style temp directory path structure with long random segment
    let custom_home = f
        .home
        .join("folders/n6/x9y8z7w6v5u4t3s2r1q0p9o8n7m6/T/home");
    let script = custom_home.join("tools/x/index.js");
    write(
        &custom_home.join(".gemini/config/mcp_config.json"),
        &format!(
            r#"{{"mcpServers": {{"x": {{"command": "node", "args": [{:?}]}}}}}}"#,
            script.to_string_lossy()
        ),
    );
    let mut cmd = f.cmd(&["inspect", "--agent", "agy"]);
    cmd.env("HOME", &custom_home);
    let (out, _, _) = f.run_cmd(&mut cmd, &["inspect", "--agent", "agy"]);
    assert!(
        out.contains(r#"args = ["{home}/tools/x/index.js"]"#),
        "{out}"
    );
}

#[test]
fn account_keys_and_secrets_never_leak() {
    let f = agy_fixture();
    for subcmd in &[
        &["probe", "--agent", "agy"][..],
        &["probe", "--agent", "agy", "--json"],
        &["inspect", "--agent", "agy"],
        &["status", "--agent", "agy"],
        &["status", "--agent", "agy", "--json"],
    ] {
        let (out, err, _) = f.run(subcmd);
        for s in SECRETS {
            assert!(!out.contains(s), "secret {s} leaked in {subcmd:?}:\n{out}");
            assert!(
                !err.contains(s),
                "secret {s} leaked in stderr of {subcmd:?}:\n{err}"
            );
        }
        assert!(
            !out.contains("oauth_token"),
            "oauth_token leaked in {subcmd:?}"
        );
    }
}

#[test]
fn skill_with_symlink_to_credential_refused() {
    let f = Fixture::new();
    write(&f.source.join("onesrc.toml"), "");
    write(
        &f.source.join("skills/my-skill/SKILL.md"),
        "---\nname: my-skill\n---\ninstructions\n",
    );
    write(&f.source.join("skills/my-skill/extra"), "source extra\n");
    write(
        &f.home.join(".gemini/config/skills/my-skill/SKILL.md"),
        "---\nname: my-skill\n---\ninstructions\n",
    );
    write(
        &f.home.join(".gemini/oauth_creds.json"),
        r#"{"oauth_token": "sk-PLANTED0secret0value0123"}"#,
    );
    symlink(
        &f.home.join(".gemini/oauth_creds.json"),
        &f.home.join(".gemini/config/skills/my-skill/extra"),
    );
    let (out, _, code) = f.run(&["status", "--agent", "agy"]);
    assert_eq!(code, 1);
    assert!(out.contains("unknown"), "{out}");
    assert!(
        out.contains("skill links into a forbidden location"),
        "{out}"
    );
    assert!(!out.contains("oauth_token"), "{out}");
    assert!(!out.contains("sk-PLANTED0secret0value0123"), "{out}");
}
