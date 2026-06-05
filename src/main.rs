//! `cargo-split-modules` — CLI entry point.
//!
//! Works both as a standalone binary (`cargo-split-modules <path>`) and as a cargo
//! subcommand (`cargo split-modules <path>`).

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

use split_modules::model::{Config, FileOutcome};
use split_modules::{split_file, split_project};

/// Split large Rust files into one-item-per-file submodules, preserving comments and
/// the public API. Every change is verified by `cargo check` and rolled back on failure.
#[derive(Parser, Debug)]
#[command(name = "cargo-split-modules", version, about, long_about = None)]
struct Cli {
    /// File to split, or a directory/crate to process recursively.
    path: PathBuf,

    /// Process a directory recursively (implied when PATH is a directory).
    #[arg(short, long)]
    recursive: bool,

    /// Show what would happen without writing any files.
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Do not run `cargo check` to verify (and roll back) the result.
    #[arg(long)]
    no_verify: bool,

    /// Do not run rustfmt on generated files.
    #[arg(long)]
    no_fmt: bool,

    /// Minimum number of resulting module files for a split to happen.
    #[arg(long, default_value_t = 2)]
    min_groups: usize,
}

fn main() -> ExitCode {
    // Support invocation as a cargo subcommand: `cargo split-modules ...`
    let mut args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) == Some("split-modules") {
        args.remove(1);
    }
    let cli = Cli::parse_from(args);

    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let config = Config {
        verify: !cli.no_verify,
        rustfmt: !cli.no_fmt,
        dry_run: cli.dry_run,
        min_groups: cli.min_groups,
    };

    if config.dry_run {
        println!("(dry run — no files will be written)");
    }
    if config.verify && !cli.dry_run {
        println!("running baseline `cargo check` …");
    }

    let recursive = cli.recursive || cli.path.is_dir();
    let mut had_problem = false;

    if recursive {
        let results = split_project(&cli.path, &config)?;
        if results.is_empty() {
            println!("no files needed splitting under {}", cli.path.display());
        }
        for (path, outcome) in &results {
            had_problem |= report(&path.display().to_string(), outcome);
        }
        let split = results
            .iter()
            .filter(|(_, o)| matches!(o, FileOutcome::Split { .. }))
            .count();
        println!("\n{} file(s) split.", split);
    } else {
        let outcome = split_file(&cli.path, &config)?;
        had_problem |= report(&cli.path.display().to_string(), &outcome);
    }

    Ok(if had_problem { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

/// Print one outcome; returns true if it represents a problem.
fn report(path: &str, outcome: &FileOutcome) -> bool {
    match outcome {
        FileOutcome::Split { files } => {
            let children = files.len().saturating_sub(1);
            println!("✓ {path} → {children} module file(s)");
            false
        }
        FileOutcome::Skipped(why) => {
            println!("• {path} skipped: {why}");
            false
        }
        FileOutcome::RolledBack(reason) => {
            eprintln!("✗ {path} rolled back (would not compile):");
            for line in reason.lines() {
                eprintln!("    {line}");
            }
            true
        }
    }
}
