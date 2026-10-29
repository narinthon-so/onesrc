# 2. Edit only owned parts of a target file

Date: 2026-10-29 · Status: accepted

## Context
Agent config files hold far more than onesrc manages (auth state, trust
lists, UI settings), and agents rewrite them while running.

## Decision
- TOML targets: byte-preserving outside owned tables (`toml_edit`, edited
  in place). Ownership by table key, not comment markers.
- JSON targets: semantic preservation only: untouched keys keep values and
  order; whitespace is re-rendered with the detected indent. Byte-identical
  output is not promised.
- Instruction files: `link` mode (symlink) or `block` mode (owned region
  between `<!-- onesrc:start -->` / `<!-- onesrc:end -->`).
- Every write: onesrc lockfile, permissions set before content, fsync,
  rename, fsync dir. Optimistic conflict detection (re-read and compare just
  before rename). This is **not** compare-and-swap: a write in the remaining
  window, or a later overwrite from an agent's cached copy, is possible.
  `sync` refuses while the agent runs (unless `--allow-running`) and
  verifies its entries afterwards.

Revised 2026-10-29 after the Codex review (see docs/reviews/).

## Consequences
- A file in an unknown shape is refused, not repaired.
- Diffs of agent configs stay small and reviewable.
