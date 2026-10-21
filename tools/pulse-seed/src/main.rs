//! One-time import of a raw dataset (Parquet tables + manifest.json)
//! into a Pulse state file, so the first sync continues incrementally instead of
//! rescanning every repository through the GitHub API.

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Parser;
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::Field;
use pulse::state_io::write_state;
use pulse_core::state::{State, Watermark};
use pulse_core::time::{Ts, format_timestamp, parse_timestamp};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

#[derive(Parser)]
#[command(about = "Import a raw Parquet dataset directory into a Pulse state file")]
struct Args {
    /// Directory with manifest.json and the *.parquet tables.
    #[arg(long)]
    raw_dir: PathBuf,
    /// State file to write (gzip when it ends with .gz).
    #[arg(long, default_value = "pulse-state.json.gz")]
    out: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let manifest: Value =
        serde_json::from_reader(File::open(args.raw_dir.join("manifest.json")).context("cannot open manifest.json")?)?;
    if manifest["schemaVersion"] != 1 || !manifest["tables"].is_array() {
        bail!("{} is not a schema 1 raw dataset", args.raw_dir.display());
    }

    let mut state = State::new();
    state.generated_at = manifest["generatedAt"].as_str().and_then(parse_timestamp).map(Ts);
    state.catalog_revision = manifest["catalogRevision"].as_str().unwrap_or_default().to_string();
    for table in manifest["tables"].as_array().into_iter().flatten() {
        let name = table["name"].as_str().unwrap_or_default();
        let path = args.raw_dir.join(table["path"].as_str().unwrap_or_default());
        match name {
            "repositories" => state.repositories = read_table(&path)?,
            "contributors" => state.contributors = read_table(&path)?,
            "pull_requests" => state.pull_requests = read_table(&path)?,
            "commits" => state.commits = read_table(&path)?,
            "pull_request_commits" => state.pull_request_commits = read_table(&path)?,
            other => eprintln!("skipping table {other}"),
        }
    }
    // The artifact's cursors become PR scan cursors. Git cursors stay empty, so the
    // first sync makes a full clone of every repository.
    for watermark in manifest["repositoryWatermarks"].as_array().into_iter().flatten() {
        state.watermarks.push(Watermark {
            repository_id: watermark["repositoryId"].as_str().unwrap_or_default().to_string(),
            pr_cursor: watermark["cursor"].as_str().and_then(parse_timestamp).map(Ts),
            git_cursor: None,
            // The dataset has no reviews; the first sync collects them with a full PR rescan.
            pr_fields_version: 0,
        });
    }

    write_state(&args.out, &state)?;
    eprintln!(
        "wrote {}: {} repositories, {} contributors, {} pull requests, {} commits, {} links, {} watermarks",
        args.out.display(),
        state.repositories.len(),
        state.contributors.len(),
        state.pull_requests.len(),
        state.commits.len(),
        state.pull_request_commits.len(),
        state.watermarks.len(),
    );
    Ok(())
}

fn read_table<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    let reader =
        SerializedFileReader::new(File::open(path).with_context(|| format!("cannot open {}", path.display()))?)?;
    let mut rows = Vec::new();
    for row in reader.get_row_iter(None)? {
        let row = row?;
        // Nulls become missing fields so the state structs apply their defaults.
        let object: Map<String, Value> = row
            .get_column_iter()
            .filter(|(_, field)| !matches!(field, Field::Null))
            .map(|(name, field)| (name.clone(), json(field)))
            .collect();
        rows.push(
            serde_json::from_value(Value::Object(object))
                .with_context(|| format!("unexpected row in {}", path.display()))?,
        );
    }
    Ok(rows)
}

fn json(field: &Field) -> Value {
    match field {
        Field::Null => Value::Null,
        Field::Bool(value) => Value::Bool(*value),
        Field::Byte(value) => (*value).into(),
        Field::Short(value) => (*value).into(),
        Field::Int(value) => (*value).into(),
        Field::Long(value) => (*value).into(),
        Field::UByte(value) => (*value).into(),
        Field::UShort(value) => (*value).into(),
        Field::UInt(value) => (*value).into(),
        Field::ULong(value) => (*value).into(),
        Field::Float(value) => f64::from(*value).into(),
        Field::Double(value) => (*value).into(),
        Field::Str(value) => Value::String(value.clone()),
        Field::TimestampMicros(value) => Value::String(format_timestamp(value.div_euclid(1_000_000))),
        Field::TimestampMillis(value) => Value::String(format_timestamp(value.div_euclid(1_000))),
        Field::Group(row) => {
            Value::Object(row.get_column_iter().map(|(name, field)| (name.clone(), json(field))).collect())
        }
        Field::ListInternal(list) => Value::Array(list.elements().iter().map(json).collect()),
        other => Value::String(other.to_string()),
    }
}
