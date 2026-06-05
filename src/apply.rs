//! Write a [`SplitPlan`] to disk with a snapshot that supports full rollback.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use crate::model::{Config, SplitPlan};
use crate::plan::{child_path, render_child};

/// A split that has been written to disk and can be rolled back.
pub struct Applied {
    pub source_path: PathBuf,
    parent_backup: String,
    created_files: Vec<PathBuf>,
    created_dir: Option<PathBuf>,
    /// Files that make up the split, for reporting.
    pub files: Vec<PathBuf>,
}

impl Applied {
    /// Undo the split: restore the parent file, delete generated files, and remove the
    /// generated directory if we created it and it is now empty.
    pub fn rollback(&self) -> Result<()> {
        std::fs::write(&self.source_path, &self.parent_backup)
            .with_context(|| format!("rollback: restoring {}", self.source_path.display()))?;
        for f in &self.created_files {
            let _ = std::fs::remove_file(f);
        }
        if let Some(dir) = &self.created_dir {
            // Only remove if empty (avoid clobbering pre-existing sibling modules).
            let _ = std::fs::remove_dir(dir);
        }
        Ok(())
    }
}

/// Write the plan to disk. Performs a conflict check first and never overwrites an
/// existing module file.
pub fn write_plan(plan: &SplitPlan, _config: &Config) -> Result<Applied> {
    // Conflict check: a generated child file must not already exist.
    for file in &plan.files {
        let path = child_path(plan, &file.stem);
        if path.exists() {
            bail!(
                "target file {} already exists; refusing to overwrite",
                path.display()
            );
        }
    }

    let parent_backup = std::fs::read_to_string(&plan.source_path)
        .with_context(|| format!("reading {}", plan.source_path.display()))?;

    // Create output directory if needed (record whether we created it).
    let created_dir = if !plan.out_dir.exists() {
        std::fs::create_dir_all(&plan.out_dir)
            .with_context(|| format!("creating {}", plan.out_dir.display()))?;
        Some(plan.out_dir.clone())
    } else {
        None
    };

    let mut created_files = Vec::new();
    let mut all_files = Vec::new();
    for file in &plan.files {
        let path = child_path(plan, &file.stem);
        let contents = render_child(plan, file);
        std::fs::write(&path, contents)
            .with_context(|| format!("writing {}", path.display()))?;
        created_files.push(path.clone());
        all_files.push(path);
    }

    std::fs::write(&plan.source_path, &plan.parent_contents)
        .with_context(|| format!("writing {}", plan.source_path.display()))?;
    all_files.push(plan.source_path.clone());

    Ok(Applied {
        source_path: plan.source_path.clone(),
        parent_backup,
        created_files,
        created_dir,
        files: all_files,
    })
}

/// Best-effort `rustfmt` on the given files; ignores all failures.
pub fn format_files(files: &[PathBuf], edition: &str) {
    if files.is_empty() {
        return;
    }
    let mut cmd = std::process::Command::new("rustfmt");
    cmd.arg("--edition").arg(edition);
    for f in files {
        cmd.arg(f);
    }
    let _ = cmd.output();
}
