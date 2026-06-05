//! End-to-end tests: build a throwaway crate in a temp dir, run the splitter, and
//! assert on both the generated structure and that the crate still compiles/tests.

use std::path::{Path, PathBuf};
use std::process::Command;

use split_modules::model::{Config, FileOutcome};
use split_modules::{split_file, split_project};
use tempfile::TempDir;

/// Create a minimal crate in a temp dir. `files` is (relative path under src, contents).
fn make_crate(files: &[(&str, &str)]) -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    for (rel, contents) in files {
        let p = root.join("src").join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, contents).unwrap();
    }
    (dir, root)
}

fn cargo_test(root: &Path) -> bool {
    Command::new("cargo")
        .arg("test")
        .arg("--quiet")
        .current_dir(root)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

#[test]
fn basic_split_preserves_api_and_compiles() {
    let big = r#"//! big module
use std::fmt;

/// A point.
#[derive(Debug, Clone, Copy)]
pub struct Point {
    pub x: i32, // x coord
    pub y: i32,
}

// leading plain comment for the impl
impl Point {
    pub fn origin() -> Self { Point { x: 0, y: 0 } }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{}", self.x, self.y)
    }
}

/// adds
pub fn add(a: i32, b: i32) -> i32 { a + b }

pub(crate) const K: i32 = 7;
"#;
    let lib = "pub mod big;\n#[test]\nfn t() { assert_eq!(big::add(big::Point::origin().x, big::K), 7); }\n";
    let (_dir, root) = make_crate(&[("big.rs", big), ("lib.rs", lib)]);

    let outcome = split_file(&root.join("src/big.rs"), &Config::default()).unwrap();
    assert!(matches!(outcome, FileOutcome::Split { .. }), "got {outcome:?}");

    // Child files exist; impls co-located with Point.
    let point = read(&root.join("src/big/point.rs"));
    assert!(point.contains("pub struct Point"));
    assert!(point.contains("impl Point"));
    assert!(point.contains("impl fmt::Display for Point"));
    // Comments preserved.
    assert!(point.contains("/// A point."));
    assert!(point.contains("// x coord"));
    assert!(point.contains("// leading plain comment for the impl"));

    // Parent re-exports preserve visibility.
    let parent = read(&root.join("src/big.rs"));
    assert!(parent.contains("pub use point::Point;"));
    assert!(parent.contains("pub use add::add;"));
    assert!(parent.contains("pub(crate) use k::K;"));
    assert!(parent.contains("mod point;"));

    assert!(cargo_test(&root), "split crate should still pass tests");
}

#[test]
fn private_member_access_from_sibling_is_preserved() {
    // `Inner` has a private field used by a *sibling* module. Moving `Inner` into a
    // child module would break that access unless we widen the field's visibility.
    let model = "pub struct Inner { secret: i32 }\npub fn make() -> Inner { Inner { secret: 41 } }\n";
    let user = "use crate::model::Inner;\npub fn read(i: &Inner) -> i32 { i.secret + 1 }\n";
    let lib = "pub mod model;\npub mod user;\n#[test]\nfn t() { assert_eq!(user::read(&model::make()), 42); }\n";
    let (_dir, root) = make_crate(&[("model.rs", model), ("user.rs", user), ("lib.rs", lib)]);

    let outcome = split_file(&root.join("src/model.rs"), &Config::default()).unwrap();
    assert!(matches!(outcome, FileOutcome::Split { .. }), "got {outcome:?}");
    let inner = read(&root.join("src/model/inner.rs"));
    assert!(inner.contains("pub(crate) secret") || inner.contains("pub(crate)  secret"));
    assert!(cargo_test(&root));
}

#[test]
fn rollback_on_break_restores_byte_identical() {
    // A `super::` path hidden inside a macro invocation is invisible to syn, so the
    // path-rewriter can't fix it; the move breaks and the tool must roll back.
    let outer = "pub fn helper() -> i32 { 7 }\npub mod m;\n";
    let m = "pub struct A;\npub fn f() -> Vec<i32> { vec![super::helper()] }\npub struct B;\n";
    let lib = "pub mod outer;\n";
    let (_dir, root) = make_crate(&[("outer.rs", outer), ("outer/m.rs", m), ("lib.rs", lib)]);

    let m_path = root.join("src/outer/m.rs");
    let before = read(&m_path);
    let outcome = split_file(&m_path, &Config::default()).unwrap();
    assert!(matches!(outcome, FileOutcome::RolledBack(_)), "got {outcome:?}");
    assert_eq!(read(&m_path), before, "file must be restored byte-identical");
    assert!(!root.join("src/outer/m").exists(), "generated dir must be removed");
}

#[test]
fn super_and_self_paths_are_rewritten() {
    // `m` defines items that reference a parent item via `super::` and a sibling via
    // `self::`. After splitting, these must be rewritten to stay valid.
    let outer = "pub fn helper() -> i32 { 100 }\npub mod m;\n";
    let m = "\
pub const SEED: i32 = 5;

pub fn via_super() -> i32 { super::helper() }

pub fn via_self() -> i32 { self::SEED + 1 }

pub fn anon_const_marker() -> i32 { 0 }
";
    let lib = "pub mod outer;\n#[test]\nfn t() { assert_eq!(outer::m::via_super(), 100); assert_eq!(outer::m::via_self(), 6); }\n";
    let (_dir, root) = make_crate(&[("outer.rs", outer), ("outer/m.rs", m), ("lib.rs", lib)]);

    let outcome = split_file(&root.join("src/outer/m.rs"), &Config::default()).unwrap();
    assert!(matches!(outcome, FileOutcome::Split { .. }), "got {outcome:?}");
    let via_super = read(&root.join("src/outer/m/via_super.rs"));
    assert!(via_super.contains("super::super::helper()"), "got: {via_super}");
    let via_self = read(&root.join("src/outer/m/via_self.rs"));
    assert!(via_self.contains("super::SEED"), "got: {via_self}");
    assert!(cargo_test(&root));
}

#[test]
fn anonymous_const_stays_in_parent() {
    let big = "\
pub struct One;
const _: () = assert!(std::mem::size_of::<u8>() == 1);
pub struct Two;
";
    let lib = "pub mod big;\n";
    let (_dir, root) = make_crate(&[("big.rs", big), ("lib.rs", lib)]);
    let outcome = split_file(&root.join("src/big.rs"), &Config::default()).unwrap();
    assert!(matches!(outcome, FileOutcome::Split { .. }), "got {outcome:?}");
    let parent = read(&root.join("src/big.rs"));
    assert!(parent.contains("const _: ()"), "anonymous const must stay in parent");
    assert!(cargo_test(&root));
}

#[test]
fn single_item_is_noop() {
    let big = "pub fn only() -> i32 { 1 }\n";
    let lib = "pub mod big;\n";
    let (_dir, root) = make_crate(&[("big.rs", big), ("lib.rs", lib)]);
    let outcome = split_file(&root.join("src/big.rs"), &Config::default()).unwrap();
    assert!(matches!(outcome, FileOutcome::Skipped(_)), "got {outcome:?}");
}

#[test]
fn recursive_splits_multiple_files() {
    let a = "pub struct One;\npub struct Two;\n";
    let b = "pub fn x() {}\npub fn y() {}\n";
    let lib = "pub mod a;\npub mod b;\n";
    let (_dir, root) = make_crate(&[("a.rs", a), ("b.rs", b), ("lib.rs", lib)]);
    let results = split_project(&root.join("src"), &Config::default()).unwrap();
    let split = results
        .iter()
        .filter(|(_, o)| matches!(o, FileOutcome::Split { .. }))
        .count();
    assert_eq!(split, 2);
    assert!(cargo_test(&root));
}

#[test]
fn dry_run_writes_nothing() {
    let big = "pub struct One;\npub struct Two;\n";
    let lib = "pub mod big;\n";
    let (_dir, root) = make_crate(&[("big.rs", big), ("lib.rs", lib)]);
    let cfg = Config { dry_run: true, ..Config::default() };
    let outcome = split_file(&root.join("src/big.rs"), &cfg).unwrap();
    assert!(matches!(outcome, FileOutcome::Split { .. }));
    assert!(!root.join("src/big").exists(), "dry run must not create files");
    assert_eq!(read(&root.join("src/big.rs")), big, "dry run must not modify source");
}
