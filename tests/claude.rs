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
      "args": ["--api-key=sk-PLANTED0secret0value0123", "ghp_PLANTED0token0value0456"],
      "env": {
        "SERVICE_TOKEN": "sk-PLANTED0secret0value0123"
      }
    },
    "handmade-http": {
      "type": "http",
      "url": "https://api.example.com/mcp?key=PLANTED0query0value0abc0123&mode=1",
      "headers": {
        "X-Api-Key": "PLANTED0header0value0789xyz"
      }
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
    write(
        &f.home.join(".claude.json"),
        r#"{"mcpServers": {"one": {"command": "one"}}}"#,
    );
    write(&f.source.join("AGENTS.md"), "rules\n");
    write(&f.home.join("alt/CLAUDE.md"), "rules\n");

    let out = f
        .cmd(&["status", "--agent", "claude"])
        .env("CLAUDE_CONFIG_DIR", f.home.join("alt"))
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
