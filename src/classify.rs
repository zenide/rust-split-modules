//! Classify each top-level item: keep it in the parent, or move it to a child
//! module file (with the visibility rewrites + re-export needed to preserve the API).
//!
//! ## Why we widen visibility
//!
//! Moving an item from module `M` into `M::child` changes which modules are
//! descendants of the item's defining module. A *private* field/method is visible to
//! `M` and all of `M`'s descendants; after the move it is only visible to
//! `M::child`'s subtree, so sibling modules that relied on the old nesting break.
//!
//! Widening such members to `pub(crate)` is a **superset** of any in-crate audience
//! they previously had, so it can never break code that used to compile, and it does
//! not change the external API (the item's *name* is re-exported at its original
//! visibility, and `pub(crate)` members remain invisible outside the crate).

use crate::util::to_snake;
use proc_macro2::Span;
use quote::quote;
use syn::spanned::Spanned;
use syn::{Attribute, Field, ImplItem, Item, Type, Visibility};

/// Result of classifying one item.
pub enum ItemClass {
    /// Stays in the parent module verbatim (`use`, `mod`, `macro_rules!`, …).
    Keep,
    /// Moves into a child file.
    Move(MoveInfo),
}

pub struct MoveInfo {
    /// Target file stem.
    pub group: String,
    /// Re-export for the parent (vis string, ident, cfg attrs). `None` for `impl`.
    pub reexport: Option<(String, String, Vec<String>)>,
    /// Visibility rewrites (item + members), in *absolute* source byte coordinates.
    pub vis_edits_abs: Vec<AbsVisEdit>,
}

/// A visibility edit in absolute source coordinates. A zero-width range is an insertion.
pub struct AbsVisEdit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Render a visibility exactly as written (empty string for inherited/private).
fn render_vis(vis: &Visibility) -> String {
    match vis {
        Visibility::Inherited => String::new(),
        v => quote!(#v).to_string(),
    }
}

/// Extract `#[cfg(...)]` / `#[cfg_attr(...)]` attributes as rendered strings.
fn cfg_attrs(attrs: &[Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
        .map(|a| quote!(#a).to_string())
        .collect()
}

fn tok_start(span: Span) -> usize {
    span.byte_range().start
}

/// Produce an edit that widens `vis` to at least `pub(crate)`, inserting at
/// `insert_at` when the visibility is currently inherited (private).
///
/// Returns `None` when no change is needed (already `pub` or `pub(crate)`).
fn widen_to_crate(vis: &Visibility, insert_at: usize) -> Option<AbsVisEdit> {
    match vis {
        Visibility::Public(_) => None,
        Visibility::Restricted(r) => {
            if r.in_token.is_none() && r.path.is_ident("crate") {
                None
            } else {
                let span = vis.span().byte_range();
                Some(AbsVisEdit { start: span.start, end: span.end, text: "pub(crate)".into() })
            }
        }
        Visibility::Inherited => Some(AbsVisEdit {
            start: insert_at,
            end: insert_at,
            text: "pub(crate) ".into(),
        }),
    }
}

/// Base identifier of a (possibly wrapped) type, used to key `impl` blocks.
fn type_base_ident(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        Type::Reference(r) => type_base_ident(&r.elem),
        Type::Paren(p) => type_base_ident(&p.elem),
        Type::Group(g) => type_base_ident(&g.elem),
        Type::Slice(s) => type_base_ident(&s.elem),
        Type::Array(a) => type_base_ident(&a.elem),
        Type::Ptr(p) => type_base_ident(&p.elem),
        _ => None,
    }
}

fn min_start(opts: &[Option<Span>], keyword: Span) -> usize {
    let mut m = tok_start(keyword);
    for o in opts.iter().flatten() {
        m = m.min(tok_start(*o));
    }
    m
}

/// Insertion offset for a field's visibility (after attributes, before the name/type).
fn field_insert_at(field: &Field) -> usize {
    match &field.ident {
        Some(id) => tok_start(id.span()),
        None => tok_start(field.ty.span()),
    }
}

/// Collect widening edits for the members of a struct/union/inherent-impl.
fn member_edits(item: &Item) -> Vec<AbsVisEdit> {
    let mut edits = Vec::new();
    match item {
        Item::Struct(it) => {
            for f in &it.fields {
                if let Some(e) = widen_to_crate(&f.vis, field_insert_at(f)) {
                    edits.push(e);
                }
            }
        }
        Item::Union(it) => {
            for f in &it.fields.named {
                if let Some(e) = widen_to_crate(&f.vis, field_insert_at(f)) {
                    edits.push(e);
                }
            }
        }
        Item::Impl(it) if it.trait_.is_none() => {
            // Inherent impl: members carry their own visibility. (Trait impls do not,
            // and adding `pub` there is a hard error — so we skip them.)
            for member in &it.items {
                match member {
                    ImplItem::Fn(f) => {
                        let insert = min_start(
                            &[
                                f.sig.constness.map(|t| t.span()),
                                f.sig.asyncness.map(|t| t.span()),
                                f.sig.unsafety.map(|t| t.span()),
                                f.sig.abi.as_ref().map(|a| a.extern_token.span()),
                            ],
                            f.sig.fn_token.span(),
                        );
                        if let Some(e) = widen_to_crate(&f.vis, insert) {
                            edits.push(e);
                        }
                    }
                    ImplItem::Const(c) => {
                        if let Some(e) = widen_to_crate(&c.vis, tok_start(c.const_token.span())) {
                            edits.push(e);
                        }
                    }
                    ImplItem::Type(t) => {
                        if let Some(e) = widen_to_crate(&t.vis, tok_start(t.type_token.span())) {
                            edits.push(e);
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    edits
}

pub fn classify(item: &Item) -> ItemClass {
    // Item + member visibility widening, plus module-relative path rewrites.
    let mut members = member_edits(item);
    members.extend(crate::pathfix::relative_path_edits(item));
    match item {
        Item::Struct(it) => named(&it.vis, &it.ident, tok_start(it.struct_token.span()), &it.attrs, members),
        Item::Enum(it) => named(&it.vis, &it.ident, tok_start(it.enum_token.span()), &it.attrs, members),
        Item::Union(it) => named(&it.vis, &it.ident, tok_start(it.union_token.span()), &it.attrs, members),
        Item::Trait(it) => {
            let insert = min_start(
                &[it.unsafety.map(|t| t.span()), it.auto_token.map(|t| t.span())],
                it.trait_token.span(),
            );
            named(&it.vis, &it.ident, insert, &it.attrs, members)
        }
        Item::TraitAlias(it) => named(&it.vis, &it.ident, tok_start(it.trait_token.span()), &it.attrs, members),
        Item::Type(it) => named(&it.vis, &it.ident, tok_start(it.type_token.span()), &it.attrs, members),
        Item::Const(it) => named(&it.vis, &it.ident, tok_start(it.const_token.span()), &it.attrs, members),
        Item::Static(it) => named(&it.vis, &it.ident, tok_start(it.static_token.span()), &it.attrs, members),
        Item::Fn(it) => {
            let insert = min_start(
                &[
                    it.sig.constness.map(|t| t.span()),
                    it.sig.asyncness.map(|t| t.span()),
                    it.sig.unsafety.map(|t| t.span()),
                    it.sig.abi.as_ref().map(|a| a.extern_token.span()),
                ],
                it.sig.fn_token.span(),
            );
            named(&it.vis, &it.sig.ident, insert, &it.attrs, members)
        }
        Item::Impl(it) => {
            let group = type_base_ident(&it.self_ty)
                .map(|s| to_snake(&s))
                .unwrap_or_else(|| "impls".to_string());
            ItemClass::Move(MoveInfo { group, reexport: None, vis_edits_abs: members })
        }
        _ => ItemClass::Keep,
    }
}

fn named(
    vis: &Visibility,
    ident: &syn::Ident,
    insert_at: usize,
    attrs: &[Attribute],
    mut edits: Vec<AbsVisEdit>,
) -> ItemClass {
    let name = ident.to_string();
    // Leave these in the parent untouched:
    //  * anonymous items (`const _: T = ...;`) — can't be re-exported by name;
    //  * `_`-prefixed items — conventionally side-effect-only (e.g. compile-time
    //    assertions), so re-exporting them just yields dead `use` warnings;
    //  * raw identifiers — awkward to re-export and rare.
    // Kept private items stay reachable from child modules via `use super::*`.
    if name.starts_with('_') || name.starts_with("r#") {
        return ItemClass::Keep;
    }
    let group = to_snake(&name);
    let reexport = Some((render_vis(vis), name, cfg_attrs(attrs)));
    if let Some(e) = widen_to_crate(vis, insert_at) {
        edits.push(e);
    }
    ItemClass::Move(MoveInfo { group, reexport, vis_edits_abs: edits })
}
