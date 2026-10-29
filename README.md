# onesrc

One source for your coding agents' instructions, skills and MCP servers.

> Status: M0 in progress — read-only `probe`, `inspect` and `status` work for
> **Codex**; Claude Code and Antigravity adapters are next. Nothing writes yet.
> See [DESIGN.md](DESIGN.md).

If you use more than one coding agent (Claude Code, OpenAI Codex CLI,
Antigravity CLI), you configure the same things once per agent, and the copies
drift. onesrc keeps one source and links or writes it into each agent, and
tells you when an agent's copy no longer matches.

- **User-level first**: your `~` config, not one repo's.
- **Ownership-safe**: edits only the entries it created; never your hand-added ones.
- **Dry-run first**: `onesrc plan` shows the change before `onesrc sync` makes it.
- **Secrets out of git**: the source holds `${VAR}`, values stay local.

## Try it (read-only)

```bash
cargo install --git https://github.com/narinthon-so/onesrc
onesrc probe               # where Codex keeps its config, what it supports
onesrc inspect             # your current setup as suggested onesrc.toml (secrets masked)
onesrc status --source ~/dotfiles/agents   # drift vs. your source; exit 0 clean, 1 drift
```

`status` reports each MCP server, skill and instructions file as `in_sync`,
`missing`, `differs`, `unmanaged`, `conflict`, `shadowed` or `unknown`.
Output never contains your username (paths print as `{home}/…`) or secret
values. `--json` gives a versioned report.

## Docs
- [DESIGN.md](DESIGN.md): problem, architecture, commands, milestones
- [docs/adr/](docs/adr/): design decisions
- [docs/reviews/](docs/reviews/): design reviews and how each point was settled

## License
MIT
