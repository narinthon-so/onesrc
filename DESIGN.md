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

~/.config/onesrc/      # never committed; dir mode 700
  secrets.env          # mode 600
  state.json           # owned entries + last-applied fingerprints
  journal/             # in-flight sync transactions
  backups/<timestamp>/ # mode 600 files, last 10 kept
  lock                 # serializes onesrc processes
```

```toml
[mcp.example-stdio]
command = "node"
args = ["{home}/tools/example-mcp/dist/index.js"]

[mcp.example-remote]
url = "https://example.com/mcp"
headers = { "X-Client" = "onesrc" }
bearer_token = "${EXAMPLE_TOKEN}"     # env reference, never a literal
targets = ["codex"]                    # optional: limit to some agents

[mcp.example-secret]
command = "npx"
args = ["some-mcp"]
env = { API_KEY = "${EXAMPLE_API_KEY}" }
```

### Values and substitution

- `${VAR}` means "the environment variable VAR". It is never a shell
  expansion. How it reaches each agent is the adapter's job (below).
- `{home}` and `{source}` are the only path substitutions. A literal `~` in
  `args` is an error: processes do not expand it.
- `command` is looked up on `PATH` at the agent's launch, as the agent does.

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
 read current → diff(desired, current, state) → Plan → apply
                         (lock, journal, backup, write, verify, commit state)
```

Adapter trait:

- `detect()` — installed? version? where is its config?
- `read()` — current MCP servers, skills, instructions
- `render(desired)` — native format, including field translation (below)
- `apply(plan)` — edit only owned entries (see "Preservation")
- `skill_strategy()` — symlink or copy, per agent
- `capabilities()` — probed facts (see "Probes")

### Field translation

The source model is agent-neutral; each adapter maps it to native fields and
refuses what it cannot express. Example, Codex (from its docs; to be
confirmed by probe): `env` with `${VAR}` values → `env_vars = ["VAR"]`
(forward by name), `bearer_token` → `bearer_token_env_var`, header values
with `${VAR}` → `env_http_headers`, literal headers → `http_headers`.
Where an agent has no env-reference field, the adapter resolves the value at
write time and the target is reported as holding a secret.

A value in `secrets.env` does not reach an agent that was launched without
it. onesrc reports which variables each target needs (`doctor`), and offers
a shell snippet that exports them; it does not manage the agent's launch.

### Skills

A skill is the **whole directory** (scripts, assets, executable bits,
agent-specific extras like `agents/openai.yaml`), never just `SKILL.md`.
Linked skills are **live by design**: editing the source changes every agent
at once, and an agent editing a linked skill edits the source repo. `status`
shows the source repo's dirty state so such edits are visible. `plan` and
`undo` cover creating and removing links, not their contents.

### Instructions: two modes

| Mode | How | Use when |
|---|---|---|
| `link` | target is a symlink to the rendered file | target is onesrc-only |
| `block` | onesrc owns only the text between `<!-- onesrc:start -->` and `<!-- onesrc:end -->` | target already has hand-written content, or the agent may rewrite the file |

`status` reports a `link` target that an agent has replaced with a regular
file (save-by-rename breaks symlinks) as drift, never silently re-links it.

### Preservation

What "untouched" means, per format:

- **TOML**: byte-preserving outside owned tables. The original `toml_edit`
  document is edited in place, never rebuilt through typed serialization.
  Ownership is by table key, not by comment markers (an agent rewriting the
  file may drop comments).
- **JSON**: semantic preservation. Untouched keys keep their values and
  order (`preserve_order`, removal by `shift_remove`); whitespace is
  re-rendered with the file's detected indent. Byte-identical output outside
  owned keys is **not** promised. Duplicate keys → refuse.

### Concurrency: what is and isn't guaranteed

Agents rewrite their own config at runtime (trust lists, state, MCP changes
from their own CLI), sometimes from a copy cached at startup.

Guaranteed:

- Only one onesrc process writes at a time (a stable lockfile, not a lock
  on the target, which rename replaces).
- No torn files: temp file beside the target, permissions set **before**
  content is written, fsync file, rename, fsync directory.
- **Optimistic conflict detection**: onesrc re-reads the target just before
  rename and aborts if it changed since planning.

Not guaranteed, and said so in `sync` output:

- A write by an agent in the window between that check and the rename.
- An agent overwriting onesrc's change later from its cached copy.

Policy: `sync` refuses while the target's agent process is running, unless
`--allow-running`; afterwards it re-reads and verifies its entries, and
reports any that did not survive.

### Ownership, transactions, undo

- `state.json` stores, per owned entry, a fingerprint of what onesrc last
  wrote. An owned entry whose current value differs from its fingerprint was
  edited by someone else → **conflict**, also on deletion.
- An unmanaged entry with the same name as a desired one → **conflict**;
  onesrc never adopts it silently.
- Each `sync` writes a journal (intent → per-target done → commit). A run
  interrupted mid-way is detected on the next run, which reports what
  landed and finishes or rolls back owned changes.
- `undo` reverses the last sync **entry by entry**, only where the current
  value still equals the post-sync fingerprint; anything changed since is
  reported and left alone. Backups exist for manual recovery, not for blind
  restore.

## Probes

Agent behaviour (paths, env-reference support, instruction precedence) is a
fact to observe, not a design decision. An agent's own description of itself
is not evidence: in review, one agent named config paths for itself that do
not exist on disk.

Two kinds:

- **Passive**: read the real home (file existence, kind, parseability,
  agent version). Never writes. Allowed in M0.
- **Behavioural**: run the agent's own commands (e.g. `mcp add`) inside a
  **disposable fixture home**, with no inherited credentials and no network,
  and observe which files change. Never against the real home.

Report: versioned JSON (plus a short human rendering). Each check has
`supported | unsupported | unknown | error` with evidence (fixture id,
sanitized argv, exit status, changed paths). Missing evidence is `unknown`,
never `unsupported`. Facts taken from documentation are marked as such,
separately from observations.

## Commands

```
onesrc inspect    # read-only: what each agent has today, as suggested onesrc.toml (redacted)
onesrc probe      # capability report (JSON or text)
onesrc status     # per agent/path/item: in_sync | missing | differs | unmanaged | conflict | shadowed | unknown
onesrc plan       # what sync would change
onesrc sync       # apply (prompts; --yes to skip)
onesrc doctor     # broken links, secrets, unset env vars, missing binaries, bad skill bundles
onesrc undo       # conditional reverse of the last sync
```

- `inspect` never writes. Adopting existing config is a human step: copy what
  you want from its output into `onesrc.toml`.
- `status` distinguishes **file-level** agreement from **effective**
  agreement: an entry can be installed and still be shadowed (e.g. Codex
  `AGENTS.override.md` beats `AGENTS.md`; a project config overrides a user
  one). It reports `shadowed` where observable and says when it cannot tell.
- Exit codes: `0` clean, `1` drift or conflict, `2` inspection error.
  `--json` output is stable and versioned.

## Safety rules

1. Ownership in per-machine `state.json` with fingerprints; hand-added
   entries are never edited or deleted. Targets are per machine, so state is
   per machine (ADR 0001).
2. Writes: lock, journal, backup, atomic replace, verify (above).
3. **Redaction from M0 on**: env values, header values and anything that
   looks like a token are masked in stdout, diffs, errors, journals and probe
   reports. `inspect` output never contains a secret value.
4. Secrets: env references where the agent supports them; otherwise resolved
   at write time into a mode-600 target, reported by `doctor` (ADR 0003).
   `doctor` checks known patterns; it cannot prove a value is not a secret.
5. Refuse when unsure: unknown shape, unknown version, unparsable file,
   duplicate keys → stop, don't "fix".
6. OAuth tokens live inside each agent; onesrc never reads or writes them.

## Tech

Rust, MSRV 1.89 (`std::fs::File::lock`). Self-contained release binaries per
OS/architecture (Linux x86_64/aarch64, macOS aarch64/x86_64). `toml_edit`
for TOML, `serde_json` with `preserve_order` for JSON.

## Testing

- Fixture `$HOME` per agent (synthetic configs, with comments); golden tests
  for render / plan / apply.
- Idempotence: `sync` twice = second run is a no-op.
- Conflict tests: target changed between plan and write → abort; owned entry
  hand-edited → conflict; same-name unmanaged entry → conflict.
- Crash test: interrupt after the first target → next run recovers.
- Redaction test: secrets in fixture configs never appear in any output.
- Filesystem edge cases: dangling and foreign symlinks, case-insensitive
  names (macOS), Unicode-equivalent names.
- CI on Linux and macOS.

## Milestones

| M | Deliverable |
|---|---|
| M0 | Passive probes + `probe`, `inspect`, `status` (read-only, redacted); schemas for `onesrc.toml`, `state.json` and the probe report; fixture-home harness |
| M1 | Instructions + skills linking; `plan` / `sync` / `undo` with lock, journal and fingerprints |
| M2 | MCP write for all three agents with field translation; behavioural probes in fixture homes |
| M3 | `doctor`, secret provisioning snippet |
| M4 | Release: README demo, comparison, binaries |
| v2 | Per-project scope, more adapters, gateway mode |
