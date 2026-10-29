# onesrc

One source for your coding agents' instructions, skills and MCP servers.

> Status: design stage. Nothing to install yet. See [DESIGN.md](DESIGN.md).

If you use more than one coding agent (Claude Code, OpenAI Codex CLI,
Antigravity CLI), you configure the same things once per agent, and the copies
drift. onesrc keeps one source and links or writes it into each agent, and
tells you when an agent's copy no longer matches.

- **User-level first**: your `~` config, not one repo's.
- **Ownership-safe**: edits only the entries it created; never your hand-added ones.
- **Dry-run first**: `onesrc plan` shows the change before `onesrc sync` makes it.
- **Secrets out of git**: the source holds `${VAR}`, values stay local.

## Docs
- [DESIGN.md](DESIGN.md): problem, architecture, commands, milestones
- [docs/adr/](docs/adr/): design decisions
- [docs/reviews/](docs/reviews/): design reviews and how each point was settled

## License
MIT
