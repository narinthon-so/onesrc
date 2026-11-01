# M0 follow-ups

Found in review of PRs #2 and #3 (2026-11-01). Not blocking M0; each is small
and can be its own PR. Reviewer: Codex.

## F1. Guard symlinks in shared skill comparison

`common::same_tree` follows every symlink inside a skill directory and reads
the target. The agy adapter now refuses skills that link into its forbidden
locations (PR #3), but the Claude and Codex adapters call `same_tree` with no
guard. A skill with `extra -> ~/.ssh/id_ed25519` would have that key read
(not printed: only compared).

- Move the walk-before-compare check into `common.rs`: refuse (`unknown`,
  content never read) when any entry's canonical target is outside the skill
  directory and outside the source repo.
- Keep agy's extra forbidden list on top.
- Tests: per adapter, a skill with a symlink to a planted file outside the
  tree; assert `unknown` and that the file's access time is unchanged.

## F2. Codex `${VAR}` label

`codex probe` reports `mcp.dollar_brace_expansion` as
`unsupported [documented]`, while Claude and agy report `unknown`. The claim
was never checked against Codex docs or behaviour. Per DESIGN "Probes",
missing evidence is `unknown`. Change it to `unknown` unless a doc citation is
added to the check.

## F3. Decide on `--version` in probe

Every adapter's `probe` runs the agent binary (`<agent> --version`) with the
real `HOME` and inherited environment, no timeout. DESIGN says passive probes
never write; an agent's startup might (logs, update checks, state files).

Decide one of: keep it with a timeout and an isolated `HOME`; read the version
from a file instead where one exists; or drop the version. Record the choice in
DESIGN "Probes".
