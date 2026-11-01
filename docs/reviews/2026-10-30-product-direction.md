# Product direction discussion and research — review brief

Date: 2026-10-30
Status: discussion record for Claude review; not an approved replacement for DESIGN.md.

## Purpose

Capture the user's product intent, decisions from the discussion, the later
architecture sketch, and existing tools found during research. Review the
direction before implementing more of the current config-sync design.

The user asked to document this discussion for Claude to review. No implementation
or external review submission is authorized by this document.

## Main problem

The user identified **redundant setup** as the main pain: configuring MCP servers,
skills, and instructions repeatedly across agents. All three matter equally.
Drift and safe edits are related concerns, but were not the user's primary reason.
No concrete incident, time cost, or frequency was established.

## Decisions explicitly selected during the config-sync discussion

These choices preceded the runtime-hub sketch below. Preserve them as decisions
from that discussion, but reassess their relevance if the architecture changes.

| Topic | Selected behavior |
|---|---|
| First usable release | Support Codex, Claude Code, and Antigravity. |
| Onboarding | Import one existing agent's setup first, with review before saving. |
| Multi-agent import | Merge later; first import uses one agent as source. |
| Skill storage | Copy whole skill bundles into the shared source; leave originals untouched during import. |
| Repeated import / same-name skills | Identical bundles: skip. Different bundles: report conflict. Compare whole bundle, including executable bits. |
| Credentials | Extract discovered literal credentials into local `secrets.env`; source uses `${VAR}` references. Preserve existing env references; OAuth remains agent-owned. Secret identification needs review because arbitrary literals may contain credentials. |
| Installation flow | `import → plan → sync`, with concrete changes previewed before explicit sync. |
| Existing conflicting target entries | Block conflicting items, continue independent items; report partial result and nonzero exit. Never silently adopt existing entries. |
| Skill portability | Copy unchanged; flag known compatibility issues; allow per-agent exclusions. No automatic skill rewriting. |
| Update behavior | Explicit `plan → sync` for installation/config changes. Linked skills/instructions update live when source content changes, with clear labeling. |

The discussion stopped before selecting how independently launched agents receive
environment variables from `secrets.env`. Options discussed were shell setup,
`onesrc run <agent>`, or resolved values in native configs. **No choice was made.**

## User's later sketch: shared runtime hub

The user drew Codex, Claude, and Agy all connecting to one `onesrc` box containing
`skill`, `mcp`, and `cli`. They gave these use cases:

1. Agents need to send email: they send something to onesrc in a standard format.
2. Agents need to open a PR.
3. Agents need to "lead skills" — interpreted in the discussion as **load skills**;
   this wording still needs confirmation.

```text
Codex ──┐
Claude ─┼── requests/results ── onesrc
Agy ────┘                        ├── skills
                                 ├── MCP backends
                                 └── CLI backends
```

This suggests centralizing access at runtime, beyond distributing config files.
The sketch is user intent; the details below are assistant proposals, not choices
the user explicitly approved.

### Proposed interpretation

- Configure providers, credentials, and skill bundles once in onesrc.
- Each agent connects to onesrc and discovers/uses shared capabilities.
- One core exposes an MCP interface for agents and a CLI interface for shell use.
- onesrc routes requests to configured MCP servers or CLI tools.
- Skills supply instructions and bundle resources; the consuming agent follows
  those instructions. A skill is not automatically an executable operation.

Illustrative names only; no schemas have been designed:

| Need | Example operation | Backend example |
|---|---|---|
| Send email | `email.send(to, subject, body)` | Email provider integration |
| Open PR | `pr.create(repo, base, head, title, body)` | GitHub MCP or `gh` |
| Load skill | `skills.get(name)` | Stored skill bundle and resources |

### Crucial unresolved distinction

**Standard protocol** and **standard action schema** are different promises:

```text
MCP aggregation:
agent → gateway → provider-specific tool and schema

Provider-neutral actions:
agent → email.send → mapping → Gmail / Outlook / CLI
```

MCP gives a common protocol. Aggregating tools does not automatically translate
different providers into one `email.send` or `pr.create` contract. The user's
"standard format" could mean either. Confirm before choosing architecture.

External actions also raise concrete execution requirements: authorization for
the requested action, clear results and errors, and duplicate prevention when
retrying sends or PR creation. These are review considerations, not implemented
guarantees or a selected approval UX.

## Relationship to the current repository

Current code implements a read-only subset of M0:

- Rust CLI: `probe`, `inspect`, `status`.
- Codex adapter only; Claude/Agy selections currently return an unimplemented error.
- Source loading, Codex MCP comparison, instruction comparison/shadow detection,
  whole-skill comparison, output masking, and versioned probe/status reports.
- Fixture-home tests and snapshots; CI configured for Linux and macOS.

Current DESIGN.md centers on config distribution, ownership, safe writes,
transactions, and undo. It lists gateway mode under v2. The new sketch may bring
gateway mode into the main product direction. Import may help populate a hub, but
the prior sync milestones should not automatically be treated as the right path
to that product.

No code was changed during the discussion. An attempted `cargo test --offline`
could not run because `cargo` was not on PATH; no test result was established.

## Existing tools found

Research date: 2026-10-30. Findings below come from project documentation and
repositories, not installation or hands-on validation. Feature completeness,
operational quality, and exact fit remain untested.

Follow-up: [Docker MCP Gateway deep dive](2026-10-30-docker-mcp-gateway.md)
covers runtime architecture, connector source findings, skills/CLI gaps, and a
proposed validation exercise. It also identifies an official MCP Skills
extension; implementation support must be verified per gateway and client.

| Tool | Documented overlap | Limits / things to verify |
|---|---|---|
| [ToolHive](https://github.com/stacklok/docs-website/blob/main/docs/toolhive/faq.mdx) | Virtual MCP Server aggregates backends behind one endpoint; centralized auth/tool filtering; skill management and registry. FAQ lists Codex, Claude Code, and Antigravity clients. | Arbitrary CLI execution needs a wrapper or adapter unless supported elsewhere. Verify local vMCP setup, skill delivery versus installation, and behavior with all three clients. |
| [MetaMCP](https://github.com/metatool-ai/metamcp) | MCP aggregation, orchestration, middleware, gateway, and tool filtering. | Skills and arbitrary CLI backends were not established in this research. Verify current maintenance/branch choice and deployment requirements. |
| [Docker MCP Gateway](https://docs.docker.com/ai/mcp-catalog-and-toolkit/mcp-gateway) | Central proxy manages configuration, credentials, routing, and MCP server lifecycle; clients connect to gateway. | Container-oriented MCP tooling. Shared skills and arbitrary CLI adapters need separate evaluation. Documentation distinguishes open-source gateway and commercial governance offerings; verify required edition/features. |
| [MCP Gateway at mcpgateway.com](https://mcpgateway.com/docs/concepts/skills) | Documents portable skill packages, import/upload, server linking, and scripts calling MCP tools through an SDK. | Verify deployment model, project identity/licensing, client-side skill loading, and CLI integration. Do not confuse this name with other projects called MCP Gateway. |

Additional ToolHive documentation:
[UI and skills entry points](https://docs.stacklok.com/toolhive/guides-ui/).

### Recommendation offered, not yet selected

Evaluate ToolHive first against the three concrete use cases before building a
generic gateway. It has substantial documented overlap. Potential remaining
onesrc value could be simpler local setup, CLI adapters, or provider-neutral
actions, but these are hypotheses requiring validation.

## Questions for Claude's review

1. Does the sketch imply a runtime gateway, config management, or a deliberate
   combination? Which part directly removes redundant setup?
2. Does "standard format" mean MCP tool calls with backend-specific schemas, or
   provider-neutral actions such as `email.send`? What is the smallest useful contract?
3. Can an existing tool satisfy email, PR creation, and skill loading across all
   three agents? Identify demonstrated gaps rather than assuming a new gateway is needed.
4. How should skills be discovered and loaded, including scripts/assets, across
   clients with different discovery behavior? Distinguish serving a skill from
   installing it and from executing its workflow.
5. Which prior import/sync decisions remain useful under the proposed hub? What
   current code can be reused, and which planned milestones should be deferred?
6. Where should credentials live, and which process needs them? Central execution
   may reduce the need to export provider secrets into each agent's environment.
7. What behavior is needed for authorization, partial failures, and retries for
   email/PR actions, without inventing an unnecessarily broad workflow engine?
8. Is there enough differentiated value to build onesrc, or should it configure or
   extend an existing gateway? Recommend a small validation exercise before a rewrite.

## Suggested review request

> Read README.md, DESIGN.md, the existing ADRs, the relevant source, and this
> discussion brief. Review whether the runtime-hub idea is the right direction
> for removing redundant setup across Codex, Claude Code, and Antigravity.
> Recheck the linked existing tools against email sending, PR creation, and skill
> loading. Separate user decisions from assistant proposals and unresolved
> assumptions. Recommend build vs extend vs adopt, with concrete gaps, tradeoffs,
> and the smallest next validation step. Do not implement or rewrite the design yet.
