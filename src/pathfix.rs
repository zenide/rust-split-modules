//! Rewrite module-relative paths inside an item that moves one level deeper.
//!
//! When an item moves from module `M` into `M::child`, any path that was written
//! relative to `M` must gain one level:
//!
//! * `super::X`  →  `super::super::X`
//! * `self::X`   →  `super::X`
//!
//! Crucially this only applies at the item's **own** module depth. A `super::` written
//! inside a nested `mod { … }` within the item is relative to that nested module and
//! must be left alone. We track `mod` nesting with a [`syn::visit::Visit`] and only
//! rewrite at depth 0. Visibilities are handled separately (see [`crate::classify`]),
//! so we skip them here to avoid double-editing. Paths hidden inside macro token
//! streams are invisible to `syn` and therefore not rewritten — if one slips through,
//! the compiler-verification step rolls the whole split back.

use syn::visit::{self, Visit};

use crate::classify::AbsVisEdit;

struct PathFixer {
    mod_depth: usize,
    edits: Vec<AbsVisEdit>,
}

impl<'ast> Visit<'ast> for PathFixer {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        self.mod_depth += 1;
        visit::visit_item_mod(self, m);
        self.mod_depth -= 1;
    }

    // Visibilities (`pub(in super::..)`) are rewritten by the visibility-widening pass;
    // don't descend into them here.
    fn visit_visibility(&mut self, _v: &'ast syn::Visibility) {}

    // Don't rewrite paths that appear inside attributes.
    fn visit_attribute(&mut self, _a: &'ast syn::Attribute) {}

    // `use super::X;` / `use self::X;` statements (e.g. inside a fn body) are not
    // `Path` nodes, so handle their leading segment explicitly.
    fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
        if self.mod_depth == 0 {
            if let syn::UseTree::Path(p) = &u.tree {
                let range = p.ident.span().byte_range();
                match p.ident.to_string().as_str() {
                    "super" => self.edits.push(AbsVisEdit {
                        start: range.start,
                        end: range.start,
                        text: "super::".into(),
                    }),
                    "self" => self.edits.push(AbsVisEdit {
                        start: range.start,
                        end: range.end,
                        text: "super".into(),
                    }),
                    _ => {}
                }
            }
        }
        visit::visit_item_use(self, u);
    }

    fn visit_path(&mut self, p: &'ast syn::Path) {
        if self.mod_depth == 0 && p.leading_colon.is_none() && p.segments.len() >= 2 {
            let first = &p.segments[0].ident;
            let range = first.span().byte_range();
            match first.to_string().as_str() {
                "super" => self.edits.push(AbsVisEdit {
                    start: range.start,
                    end: range.start,
                    text: "super::".into(),
                }),
                "self" => self.edits.push(AbsVisEdit {
                    start: range.start,
                    end: range.end,
                    text: "super".into(),
                }),
                _ => {}
            }
        }
        // Recurse so nested paths (generic args, etc.) are handled too.
        visit::visit_path(self, p);
    }
}

/// Compute the path-rewrite edits (absolute byte coords) for one moved item.
pub fn relative_path_edits(item: &syn::Item) -> Vec<AbsVisEdit> {
    let mut fixer = PathFixer { mod_depth: 0, edits: Vec::new() };
    fixer.visit_item(item);
    fixer.edits
}
