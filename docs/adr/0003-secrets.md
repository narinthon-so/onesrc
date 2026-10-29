# 3. Secrets stay out of the source

Date: 2026-10-29 · Status: accepted

## Context
MCP servers need tokens. The source directory is committed, often publicly.
Agents differ in whether their MCP config can reference environment
variables.

## Decision
- The source holds only `${VAR}` references; values live in
  `~/.config/onesrc/secrets.env` (mode 600, never committed).
- Per agent, the adapter translates `${VAR}` to the agent's native
  env-reference field (e.g. Codex forwards variables by name via `env_vars`,
  `bearer_token_env_var`, `env_http_headers`); there is no general `${VAR}`
  passthrough. With no such field: resolve at write time, mode-600 target.
- onesrc does not launch agents; `doctor` lists the variables each agent's
  environment needs and prints an export snippet.
- Redaction applies to every output from M0 on.
- OAuth tokens for remote servers are left to each agent.
- `doctor` fails on literal secrets in the source and warns on resolved
  literals in targets.

## Consequences
- Some targets will contain secret values; onesrc says which.
- Whether an agent expands `${VAR}` is verified by probe, not assumed.
