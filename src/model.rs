//! Core data types shared across the split pipeline.

use std::path::PathBuf;

/// How the parent module owns its submodule files on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// `foo.rs` → submodule files live in a sibling `foo/` directory.
    Adjacent,
    /// `lib.rs` / `main.rs` / `mod.rs` → submodule files live in the same directory.
    DirOwner,
}

/// User-facing configuration for a split run.
#[derive(Debug, Clone)]
pub struct Config {
    /// Run `cargo check` after splitting and roll back if it fails.
    pub verify: bool,
    /// Run `rustfmt` on generated/changed files when available.
    pub rustfmt: bool,
    /// Print the plan but do not touch the filesystem.
    pub dry_run: bool,
    /// In recursive mode, only split files that would yield at least this many module files.
    pub min_groups: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            verify: true,
            rustfmt: true,
            dry_run: false,
            min_groups: 2,
        }
    }
}

/// How an item's visibility must be rewritten when it moves into a child module,
/// so the parent can still re-export it at its original visibility.
#[derive(Debug, Clone)]
pub struct VisEdit {
    /// Byte range *within the item's own source slice* to replace.
    /// A zero-width range means "insert at this offset".
    pub rel_start: usize,
    pub rel_end: usize,
    /// Replacement / inserted text (e.g. `pub(super) ` or `pub(crate)`).
    pub text: String,
}

/// A re-export emitted in the parent module to preserve the original namespace.
#[derive(Debug, Clone)]
pub struct ReExport {
    /// Visibility rendered exactly as the item originally declared it.
    pub vis: String,
    /// The item's identifier.
    pub name: String,
    /// `#[cfg(...)]` attributes to replicate onto the re-export, if any.
    pub cfg_attrs: Vec<String>,
}

/// A single top-level item that will move into a child module file.
#[derive(Debug, Clone)]
pub struct MovedItem {
    /// Target file stem (snake_case); items sharing a stem share a file.
    pub group: String,
    /// Plain `//` comment block immediately preceding the item (already-doc comments
    /// are part of `text`). May be empty.
    pub leading_comment: String,
    /// Raw source slice of the item, including doc-comments and attributes.
    pub text: String,
    /// Visibility rewrites to apply to `text` for the child file (item + members).
    pub vis_edits: Vec<VisEdit>,
    /// Re-export to emit in the parent (None for `impl` blocks).
    pub reexport: Option<ReExport>,
    /// Original order index in the source file.
    pub order: usize,
}

/// One generated child module file.
#[derive(Debug, Clone)]
pub struct GroupFile {
    pub stem: String,
    /// Indices into [`SplitPlan::moved`], in original source order.
    pub item_indices: Vec<usize>,
}

/// The full plan for splitting one source file.
#[derive(Debug, Clone)]
pub struct SplitPlan {
    pub source_path: PathBuf,
    pub layout: Layout,
    /// Directory where child files are written.
    pub out_dir: PathBuf,
    /// New parent file contents (the original file rewritten as a module index).
    pub parent_contents: String,
    /// Items being moved.
    pub moved: Vec<MovedItem>,
    /// Generated files (stem + which items).
    pub files: Vec<GroupFile>,
}

impl SplitPlan {
    /// Whether this plan actually changes anything.
    pub fn is_noop(&self) -> bool {
        self.files.len() < 2
    }
}

/// Outcome of attempting to split one file.
#[derive(Debug)]
pub enum FileOutcome {
    Split { files: Vec<PathBuf> },
    Skipped(String),
    RolledBack(String),
}
