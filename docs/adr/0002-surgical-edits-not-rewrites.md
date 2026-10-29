# 2. Edit only owned parts of a target file

Date: 2026-10-29 · Status: accepted

## Context
Agent config files hold far more than onesrc manages (auth state, trust
lists, UI settings), and agents rewrite them while running.

## Decision
- TOML targets: format-preserving edit (`toml_edit`); only owned tables change.
- JSON targets: parse with key order preserved; only owned keys under the
  MCP map change; indentation is kept.
- Instruction files: `link` mode (symlink) or `block` mode (owned region
  between `<!-- onesrc:start -->` / `<!-- onesrc:end -->`).
- Every write is compare-and-swap: re-read and compare a hash right before
  the atomic rename; abort if the file changed.

## Consequences
- A file in an unknown shape is refused, not repaired.
- Diffs of agent configs stay small and reviewable.
