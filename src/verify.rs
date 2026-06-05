//! Compiler-based verification: locate the crate manifest and run `cargo check`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Walk upward from `start` to find the directory containing `Cargo.toml`.
pub fn find_manifest_dir(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_dir() {
        Some(start.to_path_buf())
    } else {
        start.parent().map(|p| p.to_path_buf())
    };
    while let Some(d) = dir {
        if d.join("Cargo.toml").is_file() {
            return Some(d);
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    None
}

/// Result of a `cargo check` run.
pub struct CheckResult {
    pub ok: bool,
    pub stderr: String,
}

/// Run `cargo check` in `manifest_dir`. `all_targets` also checks tests/examples/benches.
pub fn cargo_check(manifest_dir: &Path, all_targets: bool) -> CheckResult {
    let mut cmd = Command::new("cargo");
    cmd.arg("check").arg("--quiet").current_dir(manifest_dir);
    if all_targets {
        cmd.arg("--all-targets");
    }
    // Don't let a project-level `-D warnings` config turn cosmetic warnings into
    // rollback triggers; we only care about whether it compiles.
    cmd.env("RUSTFLAGS", "--cap-lints=warn");
    match cmd.output() {
        Ok(out) => CheckResult {
            ok: out.status.success(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        },
        Err(e) => CheckResult {
            ok: false,
            stderr: format!("failed to run cargo check: {e}"),
        },
    }
}

/// Keep only the most relevant tail of a cargo error log for reporting.
pub fn error_excerpt(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("error") || t.starts_with("-->") || t.contains("not found") || t.contains("cannot find")
        })
        .take(12)
        .collect();
    if lines.is_empty() {
        stderr.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")
    } else {
        lines.join("\n")
    }
}
