# 3. Secrets stay out of the source

Date: 2026-10-29 · Status: accepted

## Context
MCP servers need tokens. The source directory is committed, often publicly.
Agents differ in whether their MCP config can reference environment
variables.

## Decision
- The source holds only `${VAR}` references; values live in
  `~/.config/onesrc/secrets.env` (mode 600, never committed).
- Per agent, a probed capability decides: write the reference as-is when the
  agent expands it, else resolve at write time and keep the target at mode 600.
- OAuth tokens for remote servers are left to each agent.
- `doctor` fails on literal secrets in the source and warns on resolved
  literals in targets.

## Consequences
- Some targets will contain secret values; onesrc says which.
- Whether an agent expands `${VAR}` is verified by probe, not assumed.
