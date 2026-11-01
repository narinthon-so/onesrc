# M0 task: Claude Code adapter (read-only)

Implementer: agy. Reviewers: Claude and Codex, on the PR.

## Goal

`onesrc probe|inspect|status --agent claude` works like the Codex adapter
does today. Read-only: nothing under the home directory is ever written.

## Read first

- `DESIGN.md`: "Agent facts", "Probes", "Safety rules", "Commands".
- `src/agents/codex.rs`: the pattern to follow (paths, probe checks,
  inspect, status). `src/agents/common.rs` already has skill and instruction
  comparison; reuse it, do not copy it.
- `tests/codex.rs`, `tests/common/mod.rs`: fixture-home and snapshot style.

## Facts (from DESIGN.md "Agent facts")

| Item | Path |
|---|---|
| Config home | `~/.claude`, overridden by `CLAUDE_CONFIG_DIR` |
| MCP | `~/.claude.json`, top-level `mcpServers` object (user scope) |
| Instructions | `~/.claude/CLAUDE.md` (often a symlink; may contain `@path` imports) |
| Skills | `~/.claude/skills/<name>/`; entries are often symlinks to directories elsewhere |

Where `.claude.json` lives when `CLAUDE_CONFIG_DIR` is set is not confirmed.
Report it as `unknown` in `probe` rather than guessing.

`mcpServers` entries: `type` (`stdio` | `http` | `sse`; missing means stdio),
`command`, `args`, `env`, `url`, `headers`. Keep unmodelled keys as
`other_keys`, like `CodexMcp` does.

## Hard rules

1. `~/.claude.json` also holds account and session state (`oauthAccount`,
   `userID`, project history). Parse the file, read only `mcpServers`, and never
   copy, log, snapshot or print any other key. Add planted secret-like values
   in other keys to the fixture and assert they never appear in output.
2. Values in `env`, `headers` and secret-looking `args`/URLs are masked, as in
   the Codex adapter (`redact::scrub`, `MASK`).
3. Follow skill symlinks for comparison; report a dangling link as an error
   state, not a crash.
4. `@imports` in `CLAUDE.md` are not resolved in M0. Compare the file text
   only; mention imports in `probe` as a note.
5. Project-scope MCP (`projects.*.mcpServers`, `.mcp.json`) is out of scope.
   Do not report it.
6. No new dependencies unless unavoidable; say why in the PR.

## Wiring

- Add `pub mod claude;` and `AgentId::Claude` to `IMPLEMENTED` and the three
  `match`es in `src/agents/mod.rs`. The agy adapter is being written in
  parallel and touches the same lines; keep the edit minimal so the merge is
  trivial.

## Done when

- `tests/claude.rs` with a fixture home covering every `status` state the
  Codex test covers, plus: symlinked skill, dangling skill link, `CLAUDE.md`
  as a symlink, `.claude.json` with account keys and planted secrets.
- Snapshots for `probe` text, `inspect`, `status`.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test` all pass (use `--jobs 4`; this machine is memory-capped).
- One PR to `main` from branch `m0/claude-adapter`. PR body: what was built,
  every fact that was assumed rather than observed, and the test output.
