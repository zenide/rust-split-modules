//! Build a [`SplitPlan`] from a single source file: parse, classify, slice, group,
//! and render both the child files' item sources and the rewritten parent.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use syn::spanned::Spanned;

use crate::classify::{classify, ItemClass};
use crate::model::{GroupFile, Layout, MovedItem, ReExport, SplitPlan, VisEdit};
use crate::util::{extend_trailing_comment, leading_comment_start};

/// Determine how the file owns its submodules.
fn layout_of(path: &Path) -> Layout {
    match path.file_name().and_then(|s| s.to_str()) {
        Some("mod.rs") | Some("lib.rs") | Some("main.rs") => Layout::DirOwner,
        _ => Layout::Adjacent,
    }
}

/// Names already taken in the target directory / parent module, which generated
/// module stems must not collide with.
fn reserved_names(path: &Path, out_dir: &Path, layout: Layout, file: &syn::File) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    // Existing `mod x;` / `mod x { .. }` declarations and names bound by top-level
    // `use` imports (a generated `mod vec;` must not collide with `use ...::vec;`).
    for item in &file.items {
        match item {
            syn::Item::Mod(m) => {
                set.insert(m.ident.to_string());
            }
            syn::Item::Use(u) => collect_use_names(&u.tree, &mut set),
            _ => {}
        }
    }
    // Existing `.rs` files in the output directory.
    if let Ok(rd) = std::fs::read_dir(out_dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("rs") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    set.insert(stem.to_string());
                }
            }
        }
    }
    // The source file's own module stem (for DirOwner, files share the directory).
    if layout == Layout::DirOwner {
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            set.insert(stem.to_string());
        }
    }
    set
}

/// Collect the names bound by a `use` tree (final segments and `as` renames). Glob
/// imports bind unknown names and are skipped.
fn collect_use_names(tree: &syn::UseTree, set: &mut BTreeSet<String>) {
    match tree {
        syn::UseTree::Path(p) => collect_use_names(&p.tree, set),
        syn::UseTree::Name(n) => {
            set.insert(n.ident.to_string());
        }
        syn::UseTree::Rename(r) => {
            set.insert(r.rename.to_string());
        }
        syn::UseTree::Group(g) => {
            for item in &g.items {
                collect_use_names(item, set);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

/// Make `stem` unique against `reserved`, recording the chosen name as reserved.
fn unique_stem(stem: &str, reserved: &mut BTreeSet<String>) -> String {
    if !reserved.contains(stem) {
        reserved.insert(stem.to_string());
        return stem.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{stem}_{n}");
        if !reserved.contains(&candidate) {
            reserved.insert(candidate.clone());
            return candidate;
        }
        n += 1;
    }
}

/// Render the visibility + cfg prefix for a re-export line.
fn reexport_line(stem: &str, re: &ReExport) -> String {
    let mut prefix = String::new();
    for cfg in &re.cfg_attrs {
        prefix.push_str(cfg);
        prefix.push(' ');
    }
    if re.vis.is_empty() {
        format!("{prefix}use {stem}::{};", re.name)
    } else {
        format!("{prefix}{} use {stem}::{};", re.vis, re.name)
    }
}

/// Apply relative visibility edits to an item's source text (back-to-front so earlier
/// offsets stay valid).
fn apply_vis_edits(text: &str, edits: &[VisEdit]) -> String {
    let mut sorted: Vec<&VisEdit> = edits.iter().collect();
    sorted.sort_by(|a, b| b.rel_start.cmp(&a.rel_start));
    let mut out = text.to_string();
    for e in sorted {
        out.replace_range(e.rel_start..e.rel_end, &e.text);
    }
    out
}

/// Render one child module file's full contents.
pub fn render_child(plan: &SplitPlan, file: &GroupFile) -> String {
    let mut out = String::new();
    out.push_str("#[allow(unused_imports)]\nuse super::*;\n");
    for &idx in &file.item_indices {
        let item = &plan.moved[idx];
        out.push('\n');
        if !item.leading_comment.is_empty() {
            out.push_str(item.leading_comment.trim_end_matches(['\n', ' ', '\t']));
            out.push('\n');
        }
        let body = if item.vis_edits.is_empty() {
            item.text.clone()
        } else {
            apply_vis_edits(&item.text, &item.vis_edits)
        };
        out.push_str(body.trim_end());
        out.push('\n');
    }
    out
}

/// Build the split plan for `path` with the already-read `src`.
pub fn build_plan(path: &Path, src: &str) -> Result<SplitPlan> {
    let file = syn::parse_file(src)
        .with_context(|| format!("failed to parse {} as Rust", path.display()))?;

    let layout = layout_of(path);
    let parent_dir = path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
    let out_dir = match layout {
        Layout::Adjacent => {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .context("source file has no stem")?;
            parent_dir.join(stem)
        }
        Layout::DirOwner => parent_dir.clone(),
    };

    let mut reserved = reserved_names(path, &out_dir, layout, &file);

    // First pass: classify + slice, accumulating moved items keyed by raw group.
    struct Raw {
        raw_group: String,
        leading_comment: String,
        text: String,
        vis_edits: Vec<VisEdit>,
        reexport: Option<ReExport>,
        order: usize,
        delete: (usize, usize),
    }
    let mut raws: Vec<Raw> = Vec::new();
    let mut consumed_end = 0usize;

    for (order, item) in file.items.iter().enumerate() {
        let span = item.span().byte_range();
        let (start, end) = (span.start, span.end);
        let lead_start = leading_comment_start(src, consumed_end, start);
        let end_ext = extend_trailing_comment(src, end);
        consumed_end = end_ext;

        match classify(item) {
            ItemClass::Keep => {}
            ItemClass::Move(info) => {
                let leading_comment = src[lead_start..start].to_string();
                let text = src[start..end_ext].to_string();
                let vis_edits = info
                    .vis_edits_abs
                    .into_iter()
                    .map(|e| VisEdit {
                        rel_start: e.start.saturating_sub(start),
                        rel_end: e.end.saturating_sub(start),
                        text: e.text,
                    })
                    .collect();
                let reexport = info.reexport.map(|(vis, name, cfg_attrs)| ReExport {
                    vis,
                    name,
                    cfg_attrs,
                });
                raws.push(Raw {
                    raw_group: info.group,
                    leading_comment,
                    text,
                    vis_edits,
                    reexport,
                    order,
                    delete: (lead_start, end_ext),
                });
            }
        }
    }

    // Assign final, de-duplicated stems per raw group (stable in first-seen order).
    let mut stem_map: BTreeMap<String, String> = BTreeMap::new();
    // Preserve first-seen order of raw groups.
    let mut seen_order: Vec<String> = Vec::new();
    for r in &raws {
        if !stem_map.contains_key(&r.raw_group) {
            seen_order.push(r.raw_group.clone());
            stem_map.insert(r.raw_group.clone(), String::new());
        }
    }
    for raw_group in &seen_order {
        let final_stem = unique_stem(raw_group, &mut reserved);
        stem_map.insert(raw_group.clone(), final_stem);
    }

    // Materialise moved items with final stems.
    let mut moved: Vec<MovedItem> = Vec::with_capacity(raws.len());
    let mut delete_ranges: Vec<(usize, usize)> = Vec::with_capacity(raws.len());
    for r in &raws {
        let group = stem_map[&r.raw_group].clone();
        delete_ranges.push(r.delete);
        moved.push(MovedItem {
            group,
            leading_comment: r.leading_comment.clone(),
            text: r.text.clone(),
            vis_edits: r.vis_edits.clone(),
            reexport: r.reexport.clone(),
            order: r.order,
        });
    }

    // Group moved item indices by final stem, preserving original order within.
    let mut files_map: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (idx, m) in moved.iter().enumerate() {
        files_map.entry(m.group.clone()).or_default().push(idx);
    }
    let files: Vec<GroupFile> = files_map
        .into_iter()
        .map(|(stem, mut item_indices)| {
            item_indices.sort_by_key(|&i| moved[i].order);
            GroupFile { stem, item_indices }
        })
        .collect();

    // Rewrite the parent: delete moved ranges, then append mod decls + re-exports.
    let parent_contents = build_parent(src, &mut delete_ranges, &files, &moved);

    Ok(SplitPlan {
        source_path: path.to_path_buf(),
        layout,
        out_dir,
        parent_contents,
        moved,
        files,
    })
}

fn build_parent(
    src: &str,
    delete_ranges: &mut [(usize, usize)],
    files: &[GroupFile],
    moved: &[MovedItem],
) -> String {
    // Delete moved spans from the original source, working back-to-front.
    let mut body = src.to_string();
    delete_ranges.sort_by(|a, b| b.0.cmp(&a.0));
    for &(s, e) in delete_ranges.iter() {
        // Also swallow a single trailing newline left behind by the deletion.
        let mut e2 = e;
        if body[e2..].starts_with('\n') {
            e2 += 1;
        }
        body.replace_range(s..e2, "");
    }
    // Collapse 3+ consecutive blank lines that deletions may leave behind.
    let body = collapse_blank_lines(&body);

    let mut out = body.trim_end().to_string();
    out.push('\n');

    // Module declarations, sorted for determinism.
    out.push('\n');
    out.push_str("// === split-modules: generated submodules ===\n");
    let mut stems: Vec<&str> = files.iter().map(|f| f.stem.as_str()).collect();
    stems.sort_unstable();
    for stem in &stems {
        out.push_str(&format!("mod {stem};\n"));
    }

    // Re-exports, grouped per file in original item order.
    let mut any_reexport = false;
    let mut reexport_block = String::new();
    for file in files {
        for &idx in &file.item_indices {
            if let Some(re) = &moved[idx].reexport {
                reexport_block.push_str(&reexport_line(&file.stem, re));
                reexport_block.push('\n');
                any_reexport = true;
            }
        }
    }
    if any_reexport {
        out.push('\n');
        out.push_str(&reexport_block);
    }

    out
}

/// Replace runs of 3+ newlines (2+ blank lines) with a single blank line.
fn collapse_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut newline_run = 0;
    for ch in s.chars() {
        if ch == '\n' {
            newline_run += 1;
            if newline_run <= 2 {
                out.push(ch);
            }
        } else {
            newline_run = 0;
            out.push(ch);
        }
    }
    out
}

/// Child file path for a group stem under this plan.
pub fn child_path(plan: &SplitPlan, stem: &str) -> PathBuf {
    plan.out_dir.join(format!("{stem}.rs"))
}
