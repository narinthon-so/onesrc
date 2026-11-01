mod common;

use common::{symlink, write, Fixture, SECRETS};

/// A Claude home with every state `status` can report, plus:
/// symlinked skill, dangling skill link, CLAUDE.md as a symlink,
/// .claude.json with account keys and planted secrets.
fn claude_fixture() -> Fixture {
    let f = Fixture::new();
    let (h, s) = (&f.home, &f.source);

    // .claude.json with account/session keys, project-scope MCP, and planted secrets
    write(
        &h.join(".claude.json"),
        r#"{
  "oauthAccount": {
    "accountUuid": "sk-PLANTED0secret0value0123",
    "emailAddress": "PLANTED0header0value0789xyz"
  },
  "userID": "ghp_PLANTED0token0value0456",
  "projects": {
    "/synthetic/project": {
      "mcpServers": {
        "project-scoped-tool": {
          "command": "node",
          "args": ["project-tool.js"]
        }
      }
    }
  },
  "mcpServers": {
    "local-tool": {
      "command": "node",
      "args": ["/opt/tools/local-tool/index.js"]
    },
    "remote": {
      "type": "http",
      "url": "https://mcp.example.com/mcp",
      "headers": {
        "X-Client": "onesrc"
      },
      "startup_timeout_sec": 20
    },
    "drifted": {
      "command": "npx",
      "args": ["drifted-mcp", "--old-flag"]
    },
    "handmade": {
      "command": "handmade-mcp",
      "args": [
        "--api-key=sk-PLANTED0secret0value0123",
        "ghp_PLANTED0token0value0456",
        "--token",
        "PLANTED0two0arg0pw0777",
        "--token=https://example.com/PLANTED0url0key0val0secret111"
      ],
      "env": {
        "SERVICE_TOKEN": "sk-PLANTED0secret0value0123"
      }
    },
    "handmade-http": {
      "type": "http",
      "url": "https://ghp_PLANTED0user0token0222:PLANTED0userinfo0pw0999@api.example.com/mcp/ghp_PLANTED0path0token0888?key=PLANTED0query0value0abc0123&mode=1",
      "headers": {
        "X-Api-Key": "PLANTED0header0value0789xyz"
      }
    },
    "handmade-sse": {
      "type": "sse",
      "url": "https://api.example.com/sse"
    }
  }
}
"#,
    );

    write(
        &s.join("onesrc.toml"),
        r#"[mcp.local-tool]
command = "node"
args = ["/opt/tools/local-tool/index.js"]

[mcp.remote]
url = "https://mcp.example.com/mcp"
headers = { "X-Client" = "onesrc" }

[mcp.drifted]
command = "npx"
args = ["drifted-mcp"]

[mcp.not-installed]
command = "new-mcp"

[mcp.needs-env]
command = "needs-env"
env = { API_KEY = "${API_KEY}" }

[mcp.codex-only]
command = "only-codex"
targets = ["codex"]
"#,
    );

    // instructions: CLAUDE.md as a symlink
    write(&s.join("AGENTS.md"), "# Shared rules\n\nBe brief.\n");
    symlink(&s.join("AGENTS.md"), &h.join(".claude/CLAUDE.md"));

    // skills: linked, copied, drifted copy, missing, dangling, unmanaged, non-skill dir
    for name in ["linked", "copied", "drifted", "absent"] {
        write(
            &s.join(format!("skills/{name}/SKILL.md")),
            &format!("---\nname: {name}\n---\n"),
        );
    }
    write(&s.join("skills/copied/scripts/run.sh"), "echo hi\n");

    let claude_skills = h.join(".claude/skills");
    symlink(&s.join("skills/linked"), &claude_skills.join("linked"));
    write(
        &claude_skills.join("copied/SKILL.md"),
        "---\nname: copied\n---\n",
    );
    write(&claude_skills.join("copied/scripts/run.sh"), "echo hi\n");
    write(
        &claude_skills.join("drifted/SKILL.md"),
        "---\nname: drifted\n---\nlocal edit\n",
    );
    symlink(&h.join("gone/elsewhere"), &claude_skills.join("broken"));
    write(
        &claude_skills.join("extra/SKILL.md"),
        "---\nname: extra\n---\n",
    );
    write(
        &claude_skills.join("eval-workspace/notes.txt"),
        "not a skill\n",
    );

    f
}

fn stable(f: &Fixture, text: &str) -> String {
    text.replace(&f.home.to_string_lossy().to_string(), "{home}")
}

#[test]
fn status_reports_every_state() {
    let f = claude_fixture();
    let (out, err, code) = f.run(&["status", "--agent", "claude"]);
    assert_eq!(err, "");
    assert_eq!(code, 1, "drift must exit 1");
    insta::assert_snapshot!(stable(&f, &out));
}

#[test]
fn status_json_is_versioned() {
    let f = claude_fixture();
    let (out, _, code) = f.run(&["status", "--agent", "claude", "--json"]);
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
    let f = claude_fixture();
    let (out, err, code) = f.run(&["inspect", "--agent", "claude"]);
    assert_eq!((err.as_str(), code), ("", 0));
    insta::assert_snapshot!(stable(&f, &out));
}

#[test]
fn probe_text() {
    let f = claude_fixture();
    let (out, _, code) = f.run(&["probe", "--agent", "claude"]);
    assert_eq!(code, 0);
    insta::assert_snapshot!(stable(&f, &out));
}

#[test]
fn probe_json_contract() {
    let f = claude_fixture();
    let (out, _, _) = f.run(&["probe", "--agent", "claude", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let r = &v[0];
    assert_eq!(r["schema_version"], 1);
    assert_eq!(r["redacted"], true);
    assert_eq!(r["agent"]["id"], "claude");
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
        &f.home.join(".claude.json"),
        r#"{"mcpServers": {"one": {"command": "one"}}}"#,
    );
    let (out, _, code) = f.run(&["status", "--agent", "claude"]);
    assert_eq!(code, 0, "{out}");
}

#[test]
fn managed_block_counts_as_in_sync() {
    let f = Fixture::new();
    write(&f.source.join("AGENTS.md"), "rules\n");
    write(
        &f.home.join(".claude/CLAUDE.md"),
        "# my own notes\n<!-- onesrc:start -->\nrules\n<!-- onesrc:end -->\nmore notes\n",
    );
    let (out, _, code) = f.run(&["status", "--agent", "claude"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("managed block"), "{out}");
}

#[test]
fn claude_config_dir_env_is_respected() {
    let f = Fixture::new();
    write(
        &f.source.join("onesrc.toml"),
        "[mcp.one]\ncommand = \"one\"\n",
    );
    write(&f.source.join("AGENTS.md"), "rules\n");
    write(&f.source.join("skills/my-skill/SKILL.md"), "skill\n");

    // Under alt dir: instructions and skills
    write(&f.home.join("alt/CLAUDE.md"), "rules\n");
    write(&f.home.join("alt/skills/my-skill/SKILL.md"), "skill\n");

    // Even if .claude.json exists, MCP status records must be unknown
    // because location under CLAUDE_CONFIG_DIR is unconfirmed
    write(
        &f.home.join(".claude.json"),
        r#"{"mcpServers": {"one": {"command": "one"}}}"#,
    );

    let mut cmd = f.cmd(&["status", "--agent", "claude"]);
    cmd.env("CLAUDE_CONFIG_DIR", f.home.join("alt"));
    let (stdout, _, code) = f.run_cmd(&mut cmd, &["status", "--agent", "claude"]);
    assert_eq!(code, 1, "{stdout}"); // drift because MCP is unknown
    assert!(
        stdout.contains("in_sync      instructions         {home}/alt/CLAUDE.md"),
        "{stdout}"
    );
    assert!(
        stdout.contains("in_sync      my-skill             {home}/alt/skills/my-skill"),
        "{stdout}"
    );
    assert!(
        stdout.contains("unknown      one                  (unconfirmed)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("location of .claude.json when CLAUDE_CONFIG_DIR is set is unconfirmed"),
        "{stdout}"
    );

    // Probe check: probe must not present the path as fact
    let mut probe_cmd = f.cmd(&["probe", "--agent", "claude"]);
    probe_cmd.env("CLAUDE_CONFIG_DIR", f.home.join("alt"));
    let (probe_out, _, probe_code) = f.run_cmd(&mut probe_cmd, &["probe", "--agent", "claude"]);
    assert_eq!(probe_code, 0);
    assert!(
        probe_out.contains("user_config            (unconfirmed)  (other)"),
        "{probe_out}"
    );
}

#[test]
fn disabled_server_is_unknown() {
    let f = Fixture::new();
    write(
        &f.source.join("onesrc.toml"),
        "[mcp.disabled-tool]\ncommand = \"node\"\nenabled = false\n",
    );
    write(
        &f.home.join(".claude.json"),
        r#"{"mcpServers": {"disabled-tool": {"command": "node"}}}"#,
    );
    let (out, _, code) = f.run(&["status", "--agent", "claude"]);
    assert_eq!(code, 1);
    assert!(out.contains("unknown"), "{out}");
    assert!(out.contains("no confirmed disable field"), "{out}");
}

#[test]
fn duplicate_keys_in_claude_json_refused() {
    let f = Fixture::new();
    write(&f.source.join("onesrc.toml"), "");

    // duplicate mcpServers key
    write(
        &f.home.join(".claude.json"),
        r#"{"mcpServers": {}, "mcpServers": {}}"#,
    );
    let (_, err, code) = f.run(&["status", "--agent", "claude"]);
    assert_eq!(code, 2);
    assert!(err.contains("duplicate key"), "{err}");

    // duplicate server name inside mcpServers
    write(
        &f.home.join(".claude.json"),
        r#"{"mcpServers": {"srv": {"command": "a"}, "srv": {"command": "b"}}}"#,
    );
    let (_, err, code) = f.run(&["status", "--agent", "claude"]);
    assert_eq!(code, 2);
    assert!(err.contains("duplicate key"), "{err}");

    // duplicate field inside a server
    write(
        &f.home.join(".claude.json"),
        r#"{"mcpServers": {"srv": {"command": "a", "command": "b"}}}"#,
    );
    let (_, err, code) = f.run(&["status", "--agent", "claude"]);
    assert_eq!(code, 2);
    assert!(err.contains("duplicate key"), "{err}");
}

#[test]
fn inspect_shows_sse_not_as_http() {
    let f = Fixture::new();
    write(
        &f.home.join(".claude.json"),
        r#"{"mcpServers": {"sse-server": {"type": "sse", "url": "https://example.com/sse"}}}"#,
    );
    let (out, _, code) = f.run(&["inspect", "--agent", "claude"]);
    assert_eq!(code, 0);
    assert!(out.contains("[mcp.sse-server]"), "{out}");
    assert!(out.contains(r#"# not modelled: type = "sse""#), "{out}");
}

#[test]
fn instructions_differs_when_managed_block_differs() {
    let f = Fixture::new();
    write(&f.source.join("AGENTS.md"), "new rules\n");
    write(
        &f.home.join(".claude/CLAUDE.md"),
        "# custom notes\n<!-- onesrc:start -->\nold rules\n<!-- onesrc:end -->\nmore notes\n",
    );
    let (out, _, code) = f.run(&["status", "--agent", "claude"]);
    assert_eq!(code, 1);
    assert!(out.contains("differs"), "{out}");
    assert!(out.contains("managed block differs from source"), "{out}");
}

#[test]
fn instructions_conflict_when_hand_written_file_differs() {
    let f = Fixture::new();
    write(&f.source.join("AGENTS.md"), "desired rules\n");
    write(
        &f.home.join(".claude/CLAUDE.md"),
        "# completely custom rules with no managed block\n",
    );
    let (out, _, code) = f.run(&["status", "--agent", "claude"]);
    assert_eq!(code, 1);
    assert!(out.contains("conflict"), "{out}");
    assert!(
        out.contains("hand-written file without a managed block"),
        "{out}"
    );
}

#[test]
fn default_run_with_no_agent_flag() {
    let f = Fixture::new();
    write(
        &f.source.join("onesrc.toml"),
        "[mcp.tool]\ncommand = \"node\"\n",
    );
    write(
        &f.home.join(".claude.json"),
        r#"{"mcpServers": {"tool": {"command": "node"}}}"#,
    );
    write(
        &f.home.join(".codex/config.toml"),
        "[mcp_servers.tool]\ncommand = \"node\"\n",
    );

    let (out, _, code) = f.run(&["status"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("claude"), "{out}");
    assert!(out.contains("codex"), "{out}");
}

#[test]
fn dangling_skill_in_source_is_conflict() {
    let f = Fixture::new();
    write(&f.source.join("skills/linked/SKILL.md"), "skill\n");
    symlink(
        &f.home.join("nonexistent"),
        &f.home.join(".claude/skills/linked"),
    );
    let (out, _, code) = f.run(&["status", "--agent", "claude"]);
    assert_eq!(code, 1);
    assert!(out.contains("dangling symlink"), "{out}");
}

#[test]
fn dangling_instructions_symlink_is_conflict() {
    let f = Fixture::new();
    write(&f.source.join("AGENTS.md"), "rules\n");
    symlink(
        &f.home.join("nonexistent"),
        &f.home.join(".claude/CLAUDE.md"),
    );
    let (out, _, code) = f.run(&["status", "--agent", "claude"]);
    assert_eq!(code, 1);
    assert!(out.contains("dangling symlink"), "{out}");
}

#[test]
fn account_keys_and_secrets_never_leak() {
    let f = claude_fixture();
    for subcmd in &[
        &["probe", "--agent", "claude"][..],
        &["probe", "--agent", "claude", "--json"],
        &["inspect", "--agent", "claude"],
        &["status", "--agent", "claude"],
        &["status", "--agent", "claude", "--json"],
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
            !out.contains("oauthAccount"),
            "oauthAccount leaked in {subcmd:?}"
        );
        assert!(!out.contains("userID"), "userID leaked in {subcmd:?}");
        assert!(
            !out.contains("project-scoped-tool"),
            "project MCP leaked in {subcmd:?}"
        );
    }
}

#[test]
fn unexpected_shape_refused() {
    let f = Fixture::new();
    write(&f.source.join("onesrc.toml"), "");
    write(
        &f.home.join(".claude.json"),
        r#"{"mcpServers": {"x": {"args": "not-an-array"}}}"#,
    );
    let (_, err, code) = f.run(&["status", "--agent", "claude"]);
    assert_eq!(code, 2);
    assert!(err.contains("mcpServers.x.args"), "{err}");
}

#[test]
fn inspect_rewrites_home_paths() {
    let f = Fixture::new();
    let script = f.home.join("tools/x/index.js");
    write(
        &f.home.join(".claude.json"),
        &format!(
            r#"{{"mcpServers": {{"x": {{"command": "node", "args": [{:?}]}}}}}}"#,
            script.to_string_lossy()
        ),
    );
    let (out, _, _) = f.run(&["inspect", "--agent", "claude"]);
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
        &custom_home.join(".claude.json"),
        &format!(
            r#"{{"mcpServers": {{"x": {{"command": "node", "args": [{:?}]}}}}}}"#,
            script.to_string_lossy()
        ),
    );
    let mut cmd = f.cmd(&["inspect", "--agent", "claude"]);
    cmd.env("HOME", &custom_home);
    let (out, _, _) = f.run_cmd(&mut cmd, &["inspect", "--agent", "claude"]);
    assert!(
        out.contains(r#"args = ["{home}/tools/x/index.js"]"#),
        "{out}"
    );
}
