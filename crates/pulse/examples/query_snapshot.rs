//! Run dashboard API URLs against a built snapshot and save the JSON responses.
//! Usage: cargo run --release -p pulse --example query_snapshot -- <pulse.bin.gz> <urls.json> <out.json>

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use pulse::state_io::read_gzip_or_plain;
use pulse_core::snapshot::Snapshot;
use pulse_core::time::day_of;
use pulse_core::views::Dataset;
use serde_json::{Value, json};

fn main() -> anyhow::Result<()> {
    let args: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let started = std::time::Instant::now();
    let dataset = Dataset::new(Snapshot::decode(&read_gzip_or_plain(&args[0])?)?);
    eprintln!("loaded in {:?}", started.elapsed());
    let urls: Vec<String> = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let today = day_of(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64);
    let started = std::time::Instant::now();
    let results: Vec<Value> = urls
        .iter()
        .map(|url| {
            let response = match dataset.request(url, today) {
                Ok(value) => value,
                Err(error) => json!({"__error__": error.status, "detail": error.detail}),
            };
            json!({"url": url, "response": response})
        })
        .collect();
    eprintln!("{} requests in {:?}", urls.len(), started.elapsed());
    std::fs::write(&args[2], serde_json::to_vec(&results)?)?;
    Ok(())
}
