//! Checks shared by every adapter: skill bundles and instruction files.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::ctx::Ctx;
use crate::report::{ItemKind, PathKind, State, StatusRecord};
use crate::source::{AgentId, Skill, Source};

pub const BLOCK_START: &str = "<!-- onesrc:start -->";
pub const BLOCK_END: &str = "<!-- onesrc:end -->";

fn record(
    agent: AgentId,
    kind: ItemKind,
    item: &str,
    path: String,
    state: State,
    detail: Option<String>,
) -> StatusRecord {
    StatusRecord {
        agent,
        kind,
        item: item.to_string(),
        path,
        state,
        owned: false,
        detail,
    }
}

/// Status of one source skill across an agent's skill roots. The first root
/// is where onesrc would install; later roots are other places the agent
/// is known to look.
pub fn skill_status(ctx: &Ctx, agent: AgentId, skill: &Skill, roots: &[PathBuf]) -> StatusRecord {
    let found: Vec<PathBuf> = roots
        .iter()
        .map(|r| r.join(&skill.name))
        .filter(|p| PathKind::of(ctx, p) != PathKind::Missing)
        .collect();
    let Some(first) = found.first() else {
        let path = ctx.show(&roots[0].join(&skill.name));
        return record(
            agent,
            ItemKind::Skill,
            &skill.name,
            path,
            State::Missing,
            None,
        );
    };

    let (state, mut detail) = match PathKind::of(ctx, first) {
        PathKind::DanglingSymlink { target } => {
            (State::Conflict, format!("dangling symlink -> {target}"))
        }
        PathKind::Symlink { target } if same_path(first, &skill.dir) => {
            (State::InSync, format!("symlink -> {target}"))
        }
        kind @ (PathKind::Symlink { .. } | PathKind::Dir) if first.is_dir() => {
            let how = match kind {
                PathKind::Symlink { target } => format!("symlink -> {target}"),
                _ => "directory".to_string(),
            };
            match same_tree(first, &skill.dir) {
                Ok(true) => (State::InSync, format!("{how}, identical to source")),
                Ok(false) => (State::Conflict, format!("{how}, differs from source")),
                Err(e) => (State::Unknown, format!("{how}, could not compare: {e}")),
            }
        }
        _ => (
            State::Conflict,
            "exists but is not a skill directory".to_string(),
        ),
    };
    if !first.starts_with(&roots[0]) {
        detail.push_str(&format!(
            "; found in {}, not {}",
            ctx.show(first.parent().unwrap()),
            ctx.show(&roots[0])
        ));
    }
    for other in &found[1..] {
        detail.push_str(&format!(
            "; also present in {}",
            ctx.show(other.parent().unwrap())
        ));
    }
    record(
        agent,
        ItemKind::Skill,
        &skill.name,
        ctx.show(first),
        state,
        Some(detail),
    )
}

/// Skills in the agent's roots that the source does not define.
pub fn unmanaged_skills(
    ctx: &Ctx,
    agent: AgentId,
    roots: &[PathBuf],
    source: &Source,
) -> Result<Vec<StatusRecord>> {
    let known: BTreeSet<&str> = source.skills.iter().map(|s| s.name.as_str()).collect();
    let mut out = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        let mut names: Vec<(String, PathBuf)> = entries
            .filter_map(|e| e.ok())
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .filter(|(n, _)| !n.starts_with('.') && !known.contains(n.as_str()))
            .collect();
        names.sort();
        for (name, path) in names {
            let kind = PathKind::of(ctx, &path);
            let detail = match &kind {
                PathKind::DanglingSymlink { target } => {
                    Some(format!("dangling symlink -> {target}"))
                }
                PathKind::Symlink { target } if path.join("SKILL.md").is_file() => {
                    Some(format!("symlink -> {target}"))
                }
                PathKind::Dir if path.join("SKILL.md").is_file() => None,
                _ => continue, // not a skill bundle (eval workspaces, caches, …)
            };
            out.push(record(
                agent,
                ItemKind::Skill,
                &name,
                ctx.show(&path),
                State::Unmanaged,
                detail,
            ));
        }
    }
    Ok(out)
}

/// Status of the agent's global instructions file against the source.
/// `shadow` names something that overrides the file when present.
pub fn instructions_status(
    ctx: &Ctx,
    agent: AgentId,
    source: &Source,
    target: &Path,
    shadow: Option<String>,
) -> Result<Option<StatusRecord>> {
    let Some(desired) = source.instructions_for(agent)? else {
        return Ok(None);
    };
    let path = ctx.show(target);
    let (mut state, mut detail) = match PathKind::of(ctx, target) {
        PathKind::Missing => (State::Missing, None),
        PathKind::DanglingSymlink { target } => (
            State::Conflict,
            Some(format!("dangling symlink -> {target}")),
        ),
        PathKind::Symlink { target: t }
            if source
                .instructions_linkable(agent)
                .is_some_and(|src| same_path(target, src)) =>
        {
            (State::InSync, Some(format!("symlink -> {t}")))
        }
        _ => {
            let current = std::fs::read_to_string(target)?;
            match managed_block(&current) {
                Some(inner) if norm(inner) == norm(&desired) => {
                    (State::InSync, Some("managed block".into()))
                }
                Some(_) => (
                    State::Differs,
                    Some("managed block differs from source".into()),
                ),
                None if norm(&current) == norm(&desired) => {
                    (State::InSync, Some("copy, identical to source".into()))
                }
                None => (
                    State::Conflict,
                    Some(
                        "hand-written file without a managed block; onesrc would need block mode"
                            .into(),
                    ),
                ),
            }
        }
    };
    if let Some(reason) = shadow {
        if state == State::InSync {
            state = State::Shadowed;
        }
        detail = Some(match detail {
            Some(d) => format!("{d}; {reason}"),
            None => reason,
        });
    }
    Ok(Some(record(
        agent,
        ItemKind::Instructions,
        "instructions",
        path,
        state,
        detail,
    )))
}

fn managed_block(text: &str) -> Option<&str> {
    let start = text.find(BLOCK_START)? + BLOCK_START.len();
    let end = start + text[start..].find(BLOCK_END)?;
    Some(&text[start..end])
}

fn norm(s: &str) -> &str {
    s.trim_matches(|c| c == '\n' || c == '\r')
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Whole-bundle comparison: same names, same bytes, same executable bits.
pub fn same_tree(a: &Path, b: &Path) -> io::Result<bool> {
    let list = |d: &Path| -> io::Result<Vec<_>> {
        let mut v: Vec<_> = std::fs::read_dir(d)?
            .map(|e| e.map(|e| e.file_name()))
            .collect::<io::Result<_>>()?;
        v.sort();
        Ok(v)
    };
    let (la, lb) = (list(a)?, list(b)?);
    if la != lb {
        return Ok(false);
    }
    for name in la {
        let (pa, pb) = (a.join(&name), b.join(&name));
        let (ma, mb) = (std::fs::metadata(&pa)?, std::fs::metadata(&pb)?);
        if ma.is_dir() != mb.is_dir() {
            return Ok(false);
        }
        if ma.is_dir() {
            if !same_tree(&pa, &pb)? {
                return Ok(false);
            }
        } else if executable(&ma) != executable(&mb) || std::fs::read(&pa)? != std::fs::read(&pb)? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(unix)]
fn executable(m: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_: &std::fs::Metadata) -> bool {
    false
}
