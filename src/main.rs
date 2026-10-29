mod agents;
mod ctx;
mod redact;
mod report;
mod source;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};

use ctx::Ctx;
use report::{StatusReport, SCHEMA_VERSION};
use source::{AgentId, Source};

/// One source for your coding agents' instructions, skills and MCP servers.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Source directory (onesrc.toml, AGENTS.md, fragments/, skills/)
    #[arg(long, global = true, env = "ONESRC_SOURCE")]
    source: Option<PathBuf>,
    /// Limit to these agents (default: every implemented adapter)
    #[arg(long = "agent", global = true, value_enum)]
    agents: Vec<AgentArg>,
    /// Machine-readable output
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Report where each agent keeps its config and what it supports (read-only)
    Probe,
    /// Print what each agent has today as suggested onesrc.toml (read-only, redacted)
    Inspect,
    /// Compare each agent with the source (read-only). Exit 0 clean, 1 drift
    Status,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum AgentArg {
    Claude,
    Codex,
    Agy,
}

impl From<AgentArg> for AgentId {
    fn from(a: AgentArg) -> Self {
        match a {
            AgentArg::Claude => AgentId::Claude,
            AgentArg::Codex => AgentId::Codex,
            AgentArg::Agy => AgentId::Agy,
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("onesrc: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let ctx = Ctx::from_process(cli.source.clone())?;
    let agents: Vec<AgentId> = match cli.agents.is_empty() {
        true => agents::IMPLEMENTED.to_vec(),
        false => cli.agents.iter().map(|&a| a.into()).collect(),
    };

    match cli.command {
        Command::Probe => {
            let reports = agents
                .iter()
                .map(|&a| agents::probe(&ctx, a))
                .collect::<Result<Vec<_>>>()?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&reports)?);
            } else {
                for r in &reports {
                    print!("{}", report::render_probe(r));
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Inspect => {
            println!("# onesrc inspect: suggested onesrc.toml. Read-only; secrets shown as ***.");
            for &a in &agents {
                print!("\n{}", agents::inspect(&ctx, a)?);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Status => {
            let root = cli
                .source
                .as_ref()
                .context("status needs a source: pass --source or set ONESRC_SOURCE")?;
            let source = Source::load(root, &ctx.home)?;
            let ctx = Ctx::from_process(Some(source.root.clone()))?;
            let mut records = Vec::new();
            for &a in &agents {
                records.extend(agents::status(&ctx, a, &source)?);
            }
            let report = StatusReport {
                schema_version: SCHEMA_VERSION,
                records,
            };
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", report::render_status(&report));
            }
            let drift = report.records.iter().any(|r| r.state.is_drift());
            Ok(if drift {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
    }
}
