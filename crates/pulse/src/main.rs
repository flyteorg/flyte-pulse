use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};
use pulse::companies::load_company_catalog;
use pulse::config::{apply_identity_rules, load_catalog};
use pulse::snapshot_builder::{build_snapshot, write_snapshot};
use pulse::state_io::{read_state, write_state};

#[derive(Parser)]
#[command(name = "pulse", about = "Flyte Pulse collector and snapshot builder", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Update the state from Git clones and the GitHub API.
    Sync(pulse::sync::SyncArgs),
    /// Build the browser snapshot (pulse.bin.gz + meta.json) from a state file.
    Build(BuildArgs),
    /// Convert state between a .json.gz/.json file and a JSON-lines directory.
    Convert(ConvertArgs),
    /// Print a Markdown report of a built snapshot (for the GitHub Actions job summary).
    Summary(SummaryArgs),
    /// Find new companies, affiliations, and affiliation changes for review.
    Discover(pulse::discovery::DiscoverArgs),
    /// Sort companies.yaml by company ID and affiliations.yaml by username.
    FormatConfig(FormatConfigArgs),
}

#[derive(Args)]
struct FormatConfigArgs {
    /// Directory containing companies.yaml and affiliations.yaml.
    #[arg(long, default_value = "config")]
    config_dir: PathBuf,
    /// Fail if a file is not sorted instead of sorting it.
    #[arg(long)]
    check: bool,
}

fn format_config(args: FormatConfigArgs) -> Result<()> {
    let unsorted = pulse::config_format::format_config(&args.config_dir, args.check)?;
    if args.check && !unsorted.is_empty() {
        bail!("{} not sorted; run `pulse format-config`", unsorted.join(" and "));
    }
    for name in unsorted {
        eprintln!("Sorted {name}");
    }
    Ok(())
}

#[derive(Args)]
struct SummaryArgs {
    /// Directory with meta.json and the snapshot, as written by `pulse build`.
    #[arg(long, default_value = "dist/data")]
    data: PathBuf,
    /// Append the report to PATH, such as `$GITHUB_STEP_SUMMARY`, instead of printing it.
    #[arg(long, value_name = "PATH")]
    summary_file: Option<PathBuf>,
}

fn summary(args: SummaryArgs) -> Result<()> {
    let markdown = pulse::report::snapshot_markdown(&args.data, pulse::report::today())?;
    match &args.summary_file {
        Some(path) => pulse::report::append(path, &markdown),
        None => {
            print!("{markdown}");
            Ok(())
        }
    }
}

#[derive(Args)]
struct ConvertArgs {
    /// Source state: a .json.gz/.json file or a JSON-lines directory.
    #[arg(long)]
    from: PathBuf,
    /// Destination; a path without .json/.gz extension is written as a directory.
    #[arg(long)]
    to: PathBuf,
}

#[derive(Args)]
struct BuildArgs {
    #[arg(long, default_value = "pulse-state.json.gz")]
    state: PathBuf,
    /// Directory containing repositories.yaml, companies.yaml, and affiliations.yaml.
    #[arg(long, default_value = "config")]
    config_dir: PathBuf,
    /// Output directory for the snapshot files.
    #[arg(long, default_value = "dist/data")]
    out: PathBuf,
}

fn build(args: BuildArgs) -> Result<()> {
    let mut state = read_state(&args.state)?;
    // Identity rule changes take effect on the next build, without a resync.
    let catalog = load_catalog(&args.config_dir.join("repositories.yaml"))?;
    apply_identity_rules(&mut state.contributors, &catalog.identity_rules);
    let companies = load_company_catalog(&args.config_dir, &state.contributors)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
    let snapshot = build_snapshot(&state, &companies, now);
    let meta = write_snapshot(&snapshot, &args.out)?;
    eprintln!("{}", serde_json::to_string_pretty(&meta)?);
    Ok(())
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Sync(args) => pulse::sync::run(args),
        Command::Build(args) => build(args),
        Command::Convert(args) => write_state(&args.to, &read_state(&args.from)?),
        Command::Summary(args) => summary(args),
        Command::Discover(args) => pulse::discovery::run(args),
        Command::FormatConfig(args) => format_config(args),
    }
}
