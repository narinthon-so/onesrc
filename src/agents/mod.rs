//! One module per agent. Each exposes `probe`, `inspect` and `status`.

pub mod codex;
mod common;

use anyhow::{bail, Result};

use crate::ctx::Ctx;
use crate::report::{ProbeReport, StatusRecord};
use crate::source::{AgentId, Source};

/// Agents with an adapter so far.
pub const IMPLEMENTED: &[AgentId] = &[AgentId::Codex];

fn check(agent: AgentId) -> Result<()> {
    if !IMPLEMENTED.contains(&agent) {
        bail!("{agent}: adapter not implemented yet");
    }
    Ok(())
}

pub fn probe(ctx: &Ctx, agent: AgentId) -> Result<ProbeReport> {
    check(agent)?;
    Ok(match agent {
        AgentId::Codex => codex::probe(ctx),
        _ => unreachable!(),
    })
}

pub fn inspect(ctx: &Ctx, agent: AgentId) -> Result<String> {
    check(agent)?;
    match agent {
        AgentId::Codex => codex::inspect(ctx),
        _ => unreachable!(),
    }
}

pub fn status(ctx: &Ctx, agent: AgentId, source: &Source) -> Result<Vec<StatusRecord>> {
    check(agent)?;
    match agent {
        AgentId::Codex => codex::status(ctx, source),
        _ => unreachable!(),
    }
}
