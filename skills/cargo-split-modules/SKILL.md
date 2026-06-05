---
name: cargo-split-modules
description: Use when asked to split large Rust source files into smaller one-item-per-file modules, modularize a Rust crate, reduce file size, or restructure code so functions/types live in their own files (e.g. for parallel agent edits or easier search). Mechanically moves each struct/enum/trait/fn/impl into its own file, fixes imports and visibility, preserves comments and the public API, and verifies the result with `cargo check` — rolling back automatically if anything fails to compile.
---

# cargo-split-modules

`cargo-split-modules` splits large Rust files into one-item-per-file submodules. It is
**safe by construction**: it preserves the crate's public API, fixes imports/visibility,
preserves comments and doc-comments, and runs `cargo check` after every split — rolling the
change back byte-for-byte if it would not compile. You can run it without fear of breaking
the build.

## When to use this skill

- The user asks to "split this file", "break up this module", "one function/type per file",
  "modularize this crate", or "this file is too big".
- You are restructuring a Rust codebase so multiple agents can edit different items in
  parallel without merge conflicts, or so the filesystem doubles as a symbol index.
- A Rust file has grown to many hundreds/thousands of lines with many top-level items.

Only applies to **Rust** code. For a single cohesive small file, splitting adds no value —
skip it.

## How to use this skill

1. **Ensure the tool is installed** (one-time):
   ```bash
   cargo install cargo-split-modules
   ```

2. **Split a single file:**
   ```bash
   cargo split-modules path/to/big_file.rs
   ```
   `foo.rs` becomes a module index (`mod` declarations + `pub use` re-exports) and a new
   `foo/` directory holds one file per item. For `lib.rs` / `main.rs` / `mod.rs`, the new
   files are created in the same directory.

3. **Split an entire crate/directory recursively** (every file with 2+ items):
   ```bash
   cargo split-modules --recursive src
   ```

4. **Preview without writing anything:**
   ```bash
   cargo split-modules --dry-run path/to/big_file.rs
   ```

## What to tell the user / how to read the output

- `✓ <file> → N module file(s)` — split succeeded and the crate still compiles.
- `• <file> skipped: …` — nothing worth splitting (e.g. only one item).
- `✗ <file> rolled back (would not compile): …` — the split was reverted; the file is
  **unchanged**. This happens for constructs the tool can't safely move (e.g. a path hidden
  inside a macro invocation). Nothing is broken — report it and move on, or split that file
  by hand.

## Guarantees (rely on these)

- The public API is preserved: every item is re-exported from its original module at its
  original visibility, so no call sites anywhere need editing.
- Comments, doc-comments, `#[cfg(...)]`, generics, and `unsafe` are preserved verbatim.
- The compiler verifies every change; on failure the original file is restored exactly and
  generated files are removed.

## Options

```
cargo split-modules <PATH> [--recursive] [--dry-run] [--no-verify] [--no-fmt] [--min-groups N]
```

- `--recursive` / `-r`: process a directory (implied when PATH is a directory).
- `--dry-run` / `-n`: show the plan, write nothing.
- `--no-verify`: skip the `cargo check` + rollback safety step (faster, not recommended).
- `--no-fmt`: don't run rustfmt on generated files.
- `--min-groups N`: only split files that would yield at least N module files (default 2).

Source & details: https://github.com/zenide/rust-split-modules
