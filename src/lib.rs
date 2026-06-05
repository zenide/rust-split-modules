//! `split-modules`: split large Rust source files into one-item-per-file submodules,
//! preserving comments and the public API, with the compiler as the safety net.
//!
//! See [`split_file`] and [`split_project`] for the entry points.

pub mod apply;
pub mod classify;
pub mod model;
pub mod pathfix;
pub mod plan;
pub mod project;
pub mod util;
pub mod verify;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::apply::{format_files, write_plan};
use crate::model::{Config, FileOutcome};
use crate::plan::build_plan;
use crate::verify::{cargo_check, error_excerpt, find_manifest_dir};

/// Shared per-crate context resolved once (manifest dir, edition, baseline build state).
pub struct CrateCtx {
    pub manifest_dir: Option<PathBuf>,
    pub edition: String,
    /// Whether the crate compiled before we touched it (verification is meaningless if not).
    pub baseline_ok: Option<bool>,
}

impl CrateCtx {
    /// Resolve context for a path. Runs a baseline `cargo check` only when `verify`.
    pub fn resolve(path: &Path, verify: bool) -> CrateCtx {
        let manifest_dir = find_manifest_dir(path);
        let edition = manifest_dir
            .as_deref()
            .and_then(detect_edition)
            .unwrap_or_else(|| "2021".to_string());
        let baseline_ok = if verify {
            manifest_dir.as_deref().map(|d| cargo_check(d, false).ok)
        } else {
            None
        };
        CrateCtx { manifest_dir, edition, baseline_ok }
    }
}

/// Read `edition = "20xx"` from a crate manifest (best effort, no TOML dependency).
pub fn detect_edition(manifest_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest_dir.join("Cargo.toml")).ok()?;
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("edition") {
            if let Some(eq) = rest.find('=') {
                let val = rest[eq + 1..].trim().trim_matches(['"', '\'']);
                if !val.is_empty() {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}

/// Split a single file, using the supplied crate context.
pub fn split_file_with(path: &Path, config: &Config, ctx: &CrateCtx) -> Result<FileOutcome> {
    let src = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;

    let plan = build_plan(path, &src)?;
    if plan.is_noop() {
        return Ok(FileOutcome::Skipped(format!(
            "only {} module file(s) would result; nothing to split",
            plan.files.len()
        )));
    }

    if config.dry_run {
        return Ok(FileOutcome::Split {
            files: plan.files.iter().map(|f| plan.out_dir.join(format!("{}.rs", f.stem))).collect(),
        });
    }

    let applied = write_plan(&plan, config)?;

    if config.rustfmt {
        format_files(&applied.files, &ctx.edition);
    }

    let want_verify = config.verify && ctx.baseline_ok == Some(true) && ctx.manifest_dir.is_some();
    if want_verify {
        let dir = ctx.manifest_dir.as_deref().unwrap();
        let post = cargo_check(dir, false);
        if !post.ok {
            applied.rollback()?;
            return Ok(FileOutcome::RolledBack(error_excerpt(&post.stderr)));
        }
    }

    Ok(FileOutcome::Split { files: applied.files })
}

/// Split a single file (resolves its own crate context).
pub fn split_file(path: &Path, config: &Config) -> Result<FileOutcome> {
    let ctx = CrateCtx::resolve(path, config.verify);
    split_file_with(path, config, &ctx)
}

/// Split every eligible file under `root` (a file or directory).
pub fn split_project(root: &Path, config: &Config) -> Result<Vec<(PathBuf, FileOutcome)>> {
    let candidates = project::find_candidates(root, config)?;
    let ctx = CrateCtx::resolve(root, config.verify);
    let mut results = Vec::new();
    for path in candidates {
        let outcome = split_file_with(&path, config, &ctx)?;
        results.push((path, outcome));
    }
    Ok(results)
}
