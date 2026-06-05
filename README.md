# cargo-split-modules

Split large Rust source files into one-item-per-file submodules — **preserving comments
and the public API**, with the **compiler as the safety net**.

```bash
cargo install cargo-split-modules

cargo split-modules src/big.rs          # split one file
cargo split-modules --recursive src     # split every oversized file in a crate
cargo split-modules -n src/big.rs       # dry run: show what would happen
```

Turn this:

```
src/parser.rs        # 1500 lines: 12 structs, 30 fns, 20 impls
```

into this:

```
src/parser.rs        # module index: `mod` decls + `pub use` re-exports
src/parser/
    token.rs         # struct Token + its impls
    lexer.rs         # struct Lexer + its impls
    parse_expr.rs    # fn parse_expr
    ...
```

…and your crate still compiles and passes its tests, unchanged.

## Why it's safe

Most "move code around" tools risk breaking your build. This one is built so it
**cannot leave your project in a broken state**:

1. **The public API is preserved by construction.** Each item moves into a child file,
   and the parent module re-exports it at its *original visibility*
   (`pub use child::Foo;`, `pub(crate) use …`, private `use …`). Every path anywhere in
   your project that referenced `crate::parser::Token` still resolves — no call sites are
   rewritten.

2. **Children see everything via `use super::*;`.** All the original `use` imports stay
   in the parent, and child modules glob-import them along with their siblings. No
   import analysis, no guessing.

3. **Member visibility is widened safely.** Moving a struct deeper would hide its private
   fields from sibling modules that relied on the old nesting, so private members are
   widened to `pub(crate)` — a *superset* of any in-crate audience, which can never break
   compiling code and never changes the external API.

4. **Module-relative paths are rewritten with scope awareness.** `super::X` →
   `super::super::X` and `self::X` → `super::X`, but only at the item's own module depth
   (paths inside nested `mod {}` blocks are left alone).

5. **The compiler verifies every split.** After writing files, `cargo split-modules` runs
   `cargo check`. If anything fails to compile, it **rolls back the entire split**,
   restoring the original file byte-for-byte and removing generated files. You either get
   a working split or no change at all.

This has been validated by splitting real crates end to end and confirming their full test
suites still pass — see [Validation](#validation-on-real-crates) below.

## Validation on real crates

Each crate below was cloned, split recursively (`--recursive src`), and had its **own** test
suite run before and after. In every case the test counts are identical — behaviour is
preserved. The few files that couldn't be split safely were rolled back automatically and
left untouched.

| crate | files before → after | avg LOC/file before → after | files split | rolled back | tests before → after |
| --- | --- | --- | --- | --- | --- |
| semver | 8 → 65 | 264 → 36 | 8 | 0 | 38 → 38 ✅ |
| bytes | 19 → 129 | 518 → 79 | 9 | 0 | 1303 → 1303 ✅ |
| anyhow | 12 → 64 | 326 → 64 | 9 | 1 | 96 → 96 ✅ |
| httparse | 9 → 65 | 457 → 67 | 5 | 2 | 368 → 368 ✅ |
| base64 | 21 → 198 | 340 → 39 | 16 | 0 | 222 → 222 ✅ |
| memchr | 45 → 223 | 350 → 74 | 28 | 0 | 136 → 136 ✅ |
| bitflags | 44 → 128 | 133 → 49 | 31 | 0 | 74 → 74 ✅ |
| heck | 9 → 43 | 96 → 23 | 9 | 0 | 128 → 128 ✅ |
| **total** | **167 → 915** | **297 → 54** | **115** | **3** | **2365 → 2365 ✅** |

Across ~50k lines of third-party code, 115 files were split and **not one test changed its
result** — the 3 unsplittable files were safely rolled back.

## What gets preserved

- Doc-comments (`///`, `//!`) and `#[derive]`/attribute lines — they're part of each
  item's span and move with it.
- Plain `//` comments directly above an item, and trailing same-line comments.
- `#[cfg(...)]` attributes — replicated onto the generated re-export.
- Generics, `unsafe`, `async`, lifetimes, `where` clauses — the item's source text is
  sliced verbatim, never reformatted away.

## How items are grouped

One file per item, named after it (snake_case):

| Item | Goes to |
| --- | --- |
| `struct` / `enum` / `union` / `type` / `trait` | `name.rs` |
| free `fn` | `name.rs` |
| `const` / `static` | `name.rs` |
| `impl Foo` / `impl Trait for Foo` | co-located in `foo.rs` (with `Foo`) |

A same-named const, type alias, and struct merge into one file. `impl` blocks for an
external/complex self type land in `impls.rs`.

Things that **stay in the parent**: `use`, `mod`, `extern crate`, `macro_rules!`,
anonymous (`const _`) and `_`-prefixed side-effect items.

## File layout

- `foo.rs` → a sibling `foo/` directory is created and `foo.rs` becomes the module index.
- `lib.rs` / `main.rs` / `mod.rs` → generated files go in the *same* directory (these
  already own a directory module).

## Options

```
cargo split-modules <PATH> [OPTIONS]

  PATH                 A .rs file to split, or a directory/crate to process recursively.

  -r, --recursive      Process a directory recursively (implied when PATH is a directory).
                       Splits every file that would yield 2+ module files.
  -n, --dry-run        Show what would happen without writing anything.
      --no-verify      Skip the cargo check + rollback safety step (faster, not advised).
      --no-fmt         Don't run rustfmt on generated files.
      --min-groups N   Minimum number of resulting module files for a split (default 2).
```

## Known limitations (handled by safe rollback)

A file is **safely skipped** (rolled back, never broken) when a split would not compile —
in practice this means paths hidden inside macro token streams (`some_macro!(super::X)`),
or other constructs the AST can't see. You lose nothing: the file is left exactly as it
was, and the tool tells you which files it skipped.

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.

[`semver`]: https://crates.io/crates/semver
[`bytes`]: https://crates.io/crates/bytes
