# onesrc — design

Status: draft (2026-10-29), revised after review —
see `docs/reviews/`

## Problem

People who use more than one coding agent (Claude Code, OpenAI Codex CLI,
Google Antigravity CLI `agy`) configure the same things once per agent:
instructions, skills, MCP servers, and the secrets those servers need. The
copies drift. A skill fixed in one agent stays stale in the others; an MCP
server added to one agent is missing from the rest; an instructions file is a
hand-made copy that nobody remembers to update.

## One-liner

Write your agent setup once — instructions, skills, MCP servers, secrets —
and `onesrc` links or writes it into every agent, and reports drift before it
bites.

## Goals / non-goals

| Goals | Non-goals (v1) |
|---|---|
| User-level (`~`) config first | Hooks and permission allowlists (formats too different) |
| Never touch entries onesrc did not create | Session/transcript sync |
| Dry-run by default: `plan`, then `sync` | GUI |
| Secrets never in the source directory | Every agent; adapters are added one at a time |
| Same source on Linux (incl. WSL) and macOS | Per-project scope (v2) |

## Prior art

| Tool | Gap for this use case |
|---|---|
| amtiYo/agents | Project-centric `.agents/`; global paths per agent not documented |
| eggboy/mcpshare | MCP only; no skills or instructions |
| spyrae/agentsync | Claude is source only; no skills |

onesrc's niche: user-level, all three kinds of config, ownership-safe writes.

## Source layout

```
<source dir, e.g. ~/dotfiles/agents>/   # under version control
  onesrc.toml          # MCP servers, targets, per-agent overrides
  AGENTS.md            # shared instructions
  fragments/<agent>.md # agent-only additions
  skills/<name>/SKILL.md

~/.config/onesrc/      # never committed
  secrets.env          # mode 600
  state.json           # what onesrc owns in each target
  backups/<timestamp>/
```

```toml
[mcp.example-stdio]
command = "node"
args = ["~/tools/example-mcp/dist/index.js"]

[mcp.example-remote]
url = "https://example.com/mcp"
targets = ["codex"]                  # optional: limit to some agents

[mcp.example-secret]
command = "npx"
args = ["some-mcp"]
env = { API_KEY = "${EXAMPLE_API_KEY}" }   # resolved from secrets.env
```

## Architecture

```
       source (toml + md + skills)
                  │ load + validate
                  ▼
          Desired state (model)
                  │
   ┌──────────────┼──────────────┐
   ▼              ▼              ▼
ClaudeAdapter  CodexAdapter   AgyAdapter        ← one trait
   │              │              │
 read current → diff(desired, current, owned) → Plan → apply
                                     (backup, atomic write, update state)
```

Adapter trait:

- `detect()` — installed? where is its config?
- `read()` — current MCP servers, skills, instructions
- `render(desired)` — native format
- `apply(plan)` — surgical edit: TOML via a format-preserving editor
  (comments survive), JSON touching only owned `mcpServers` keys
- `skill_strategy()` — symlink or copy, per agent
- `capabilities()` — probed facts: env-reference syntax, remote server
  fields (`url`, headers), disabled flag, instruction file location

Exact target paths per agent are **facts to probe**, not design decisions.
An agent's own description of its paths is not a probe: during review, one
agent named config paths for itself that do not exist on disk. A probe is an
observation, e.g. run the agent's own `mcp add` and see which file changed.

### Instructions: two modes

| Mode | How | Use when |
|---|---|---|
| `link` | target is a symlink to the rendered file | target is onesrc-only |
| `block` | onesrc owns only the text between `<!-- onesrc:start -->` and `<!-- onesrc:end -->` | target already has hand-written content, or the agent may rewrite the file |

`status` reports a `link` target that an agent has replaced with a regular
file (save-by-rename breaks symlinks) as drift, never silently re-links it.

### Writing while an agent runs

Agents rewrite their own config at runtime (state, trust lists, MCP changes
from their own CLI). Every write therefore:

1. reads the target and records a hash;
2. computes the edit;
3. re-reads and compares the hash right before the atomic rename; if it
   changed, aborts and asks for a re-run;
4. warns if the agent's process is running.

M0's read-only `status` exists partly to confirm the probed facts on real
machines.

## Commands

```
onesrc inspect    # read-only: what each agent has today, printed as suggested onesrc.toml
onesrc status     # drift: missing / differs / unmanaged
onesrc plan       # what sync would change
onesrc sync       # apply (prompts; --yes to skip)
onesrc doctor     # broken links, literal secrets, unset env vars, missing binaries, bad SKILL.md frontmatter
onesrc undo       # restore the last backup
```

`inspect` never writes. Adopting existing config is a human step: copy what
you want from its output into `onesrc.toml`. An automatic import would need
three-way conflict resolution, and a no-op round trip is not realistic once
agents disagree on args, env or wording.

## Safety rules

1. Ownership in `state.json`; hand-added entries are never edited or deleted.
   State is per machine, because targets are per machine: two machines
   sharing one source each record what they wrote into their own files.
   Entries removed from the source are cleaned up from local state.
2. Every write: backup, compare-and-swap (above), atomic rename keeping file
   mode.
3. Secrets: use the agent's native env-reference syntax where its
   `capabilities()` says one exists; otherwise resolve at write time, keep the
   target at mode 600, and `doctor` warns that the target holds a literal.
4. Refuse when unsure: unknown shape, unknown version, unparsable file → stop,
   don't "fix".
5. Remote MCP servers carry `url` and optional `headers` (values may be
   `${VAR}`). OAuth tokens live inside each agent; onesrc never reads or
   writes them.

## Tech

Rust: one static binary for Linux and macOS; `toml_edit` for
format-preserving TOML; `serde_json` with `preserve_order` for JSON targets
(the targets have no comments, but key order and indentation must survive so
diffs stay small).

## Testing

- Fixture `$HOME` per agent (synthetic configs, with comments); golden tests
  for render / plan / apply.
- Idempotence: `sync` twice = second run is a no-op.
- Race test: target modified between read and write → abort, nothing written.
- CI on Linux and macOS.

## Milestones

| M | Deliverable |
|---|---|
| M0 | Probes + `inspect` + `status` (read-only); `onesrc.toml` and `state.json` schemas |
| M1 | Instructions + skills linking; `plan` / `sync` / `undo` |
| M2 | MCP write for all three agents, with ownership |
| M3 | Secrets + `doctor` |
| M4 | Release: README demo, comparison, ADRs, binaries |
| v2 | Per-project scope, more adapters, gateway mode |
