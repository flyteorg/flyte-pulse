use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use pulse::companies::load_company_catalog;
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
    /// Directory containing companies.yaml and affiliations.yaml.
    #[arg(long, default_value = "config")]
    config_dir: PathBuf,
    /// Output directory for the snapshot files.
    #[arg(long, default_value = "dist/data")]
    out: PathBuf,
}

fn build(args: BuildArgs) -> Result<()> {
    let state = read_state(&args.state)?;
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
    }
}
