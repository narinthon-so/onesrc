# M0 task: Antigravity CLI (`agy`) adapter (read-only)

Implementer: goose. Reviewers: Claude and Codex, on the PR.

## Goal

`onesrc probe|inspect|status --agent agy` works like the Codex adapter does
today. Read-only: nothing under the home directory is ever written.

## Read first

- `DESIGN.md`: "Agent facts", "Probes", "Safety rules", "Commands".
- `src/agents/codex.rs`: the pattern to follow (paths, probe checks,
  inspect, status). `src/agents/common.rs` already has skill and instruction
  comparison; reuse it, do not copy it.
- `tests/codex.rs`, `tests/common/mod.rs`: fixture-home and snapshot style.

## Facts (from DESIGN.md "Agent facts")

| Item | Path | Source |
|---|---|---|
| MCP | `~/.gemini/config/mcp_config.json`, `mcpServers` object | observed: `agy mcp add` writes here (agy 1.4.12) |
| Instructions | `~/.gemini/GEMINI.md`, `~/.gemini/AGENTS.md`, and the same names in `~/.gemini/config/` | doc embedded in agy |
| Skills | `~/.gemini/config/skills/<name>/` | doc embedded in agy |

Observed MCP entry shapes, as written by `agy mcp add`:

```json
{
  "mcpServers": {
    "local": { "command": "echo", "args": ["hand"], "disabled": false },
    "remote": { "serverUrl": "http://127.0.0.1:4483/mcp", "disabled": false }
  }
}
```

`env` and `headers` are accepted by `agy mcp add` (`--env`, `--header`); their
JSON key names are not confirmed. Treat `env` and `headers` as the likely
names, and keep anything else as `other_keys`.

Not the agy CLI's files, and must not be read as such:
`~/.gemini/antigravity/mcp_config.json` (Antigravity IDE),
`~/.gemini/settings.json` (Gemini CLI), `~/.gemini/antigravity-cli/` (agy's own
internal state, which includes an OAuth token file).

## Hard rules

1. Never read anything under `~/.gemini/antigravity-cli/`, and never read
   `~/.gemini/oauth_creds.json`, `google_accounts.json` or
   `gemini-credentials.json`. `probe` may report that they exist (path kind
   only), never their content.
2. Values in `env`, `headers` and secret-looking `args`/URLs are masked, as in
   the Codex adapter (`redact::scrub`, `MASK`). Plant secrets in the fixture
   and assert they never appear in output.
3. Several instruction files can be active at once. Report every one that
   exists. If more than one exists, mark the extras `shadowed` or add a probe
   note; do not guess precedence. Say which you chose in the PR.
4. `disabled: true` maps to the existing enabled/disabled handling.
5. No new dependencies unless unavoidable; say why in the PR.

## Wiring

- Add `pub mod agy;` and `AgentId::Agy` to `IMPLEMENTED` and the three
  `match`es in `src/agents/mod.rs`. The Claude adapter is being written in
  parallel and touches the same lines; keep the edit minimal so the merge is
  trivial.

## Done when

- `tests/agy.rs` with a fixture home covering every `status` state the Codex
  test covers, plus: an HTTP server with `serverUrl`, a disabled server, two
  instruction files at once, and an IDE file at
  `~/.gemini/antigravity/mcp_config.json` that must be ignored.
- Snapshots for `probe` text, `inspect`, `status`.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test` all pass (use `--jobs 4`; this machine is memory-capped).
- One PR to `main` from branch `m0/agy-adapter`. PR body: what was built,
  every fact that was assumed rather than observed, and the test output.
