# 1. Ownership is tracked per machine, outside the source

Date: 2026-10-29 · Status: accepted

## Context
onesrc edits files other tools also edit (agent configs). It must know which
entries it created, so it can update and remove them without touching
hand-added ones. The source directory is usually shared between machines via
git.

## Decision
Record owned entries in `~/.config/onesrc/state.json` on each machine, keyed
by target file and entry. Never in the source repo, never as markers inside
JSON targets.

## Consequences
- Each machine's state describes exactly the files on that machine.
- A lockfile in the source repo (suggested in review) was rejected: two
  machines would overwrite each other's record of files the other cannot see.
- Losing `state.json` makes onesrc treat its old entries as unmanaged; it
  will not delete them. `status` reports them so the user can adopt them.
