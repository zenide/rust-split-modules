//! Recursive project mode: find `.rs` files that violate the one-item-per-file rule.

use std::path::{Path, PathBuf};

use anyhow::Result;
use walkdir::WalkDir;

use crate::model::Config;
use crate::plan::build_plan;

/// Directories never worth descending into.
fn is_ignored_dir(name: &str) -> bool {
    matches!(name, "target" | ".git" | "node_modules") || name.starts_with('.')
}

/// Would splitting `path` produce at least `min_groups` module files?
fn is_candidate(path: &Path, min_groups: usize) -> bool {
    let Ok(src) = std::fs::read_to_string(path) else {
        return false;
    };
    match build_plan(path, &src) {
        Ok(plan) => plan.files.len() >= min_groups,
        Err(_) => false, // unparseable / non-Rust: leave it alone
    }
}

/// Collect all eligible files under `root`. If `root` is a single `.rs` file, it is the
/// only candidate (subject to the same threshold).
pub fn find_candidates(root: &Path, config: &Config) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if root.is_file() {
        if is_candidate(root, config.min_groups) {
            out.push(root.to_path_buf());
        }
        return Ok(out);
    }

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            // Prune ignored directories.
            if e.file_type().is_dir() {
                if let Some(name) = e.file_name().to_str() {
                    if e.depth() > 0 && is_ignored_dir(name) {
                        return false;
                    }
                }
            }
            true
        })
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        if is_candidate(path, config.min_groups) {
            out.push(path.to_path_buf());
        }
    }

    out.sort();
    Ok(out)
}
