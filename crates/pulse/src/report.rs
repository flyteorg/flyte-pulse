//! Markdown reports for the GitHub Actions job summary.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

use anyhow::{Context, Result};
use pulse_core::snapshot::Snapshot;
use pulse_core::time::{day_of, format_date, format_timestamp, parse_timestamp};
use pulse_core::views::Dataset;
use serde_json::Value;

use crate::state_io::read_gzip_or_plain;

/// `1234567` → `1,234,567`
pub fn thousands(value: impl Into<i128>) -> String {
    let value = value.into();
    let digits = value.unsigned_abs().to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    if value < 0 { format!("-{out}") } else { out }
}

/// `185` → `3m 5s`
pub fn duration(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {}s", seconds / 60, seconds % 60),
        _ => format!("{}h {}m", seconds / 3600, seconds % 3600 / 60),
    }
}

/// `2930348` → `2.8 MB`
pub fn bytes(value: u64) -> String {
    match value {
        0..1024 => format!("{value} B"),
        1024..1_048_576 => format!("{:.0} KB", value as f64 / 1024.0),
        _ => format!("{:.1} MB", value as f64 / 1_048_576.0),
    }
}

/// `2026-10-18T22:44:34Z` → `2026-10-18 22:44 UTC`
pub fn utc(value: &str) -> String {
    parse_timestamp(value)
        .map(|t| format_timestamp(t).replace('T', " ")[..16].to_string() + " UTC")
        .unwrap_or_else(|| value.to_string())
}

/// Append Markdown to a file such as `$GITHUB_STEP_SUMMARY`.
pub fn append(path: &Path, markdown: &str) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("cannot open summary file {}", path.display()))?;
    file.write_all(markdown.as_bytes())?;
    Ok(())
}

fn number(value: &Value) -> String {
    thousands(value.as_i64().unwrap_or(0))
}

/// The published site data: snapshot size and recent activity, computed with the
/// same queries the browser runs.
pub fn snapshot_markdown(data_dir: &Path, today: i32) -> Result<String> {
    let meta: Value = serde_json::from_slice(
        &std::fs::read(data_dir.join("meta.json")).context("cannot read meta.json; run `pulse build` first")?,
    )?;
    let file = meta["file"].as_str().unwrap_or("pulse.bin.gz");
    let dataset = Dataset::new(Snapshot::decode(&read_gzip_or_plain(&data_dir.join(file))?)?);
    let query = |url: &str| dataset.request(url, today).map_err(|e| anyhow::anyhow!("{url}: {}", e.detail));

    let mut out = String::new();
    writeln!(out, "## Site data\n")?;
    writeln!(
        out,
        "Snapshot `{}`, a {} download ({} uncompressed), built {} from data collected {}.\n",
        meta["version"].as_str().unwrap_or("unknown"),
        bytes(meta["size"].as_u64().unwrap_or(0)),
        bytes(meta["uncompressed_size"].as_u64().unwrap_or(0)),
        utc(meta["generated_at"].as_str().unwrap_or("")),
        utc(meta["raw_generated_at"].as_str().unwrap_or("")),
    )?;

    let periods = [7, 30, 365];
    let summaries: Vec<Value> = periods
        .iter()
        .map(|days| query(&format!("/api/dashboard?days={days}")).map(|v| v["summary"].clone()))
        .collect::<Result<_>>()?;
    writeln!(out, "| Activity until {} | Last 7 days | Last 30 days | Last 365 days |", format_date(today))?;
    writeln!(out, "|---|--:|--:|--:|")?;
    for (label, key) in [
        ("Pull requests merged", "pull_requests_merged"),
        ("Pull requests opened", "pull_requests_opened"),
        ("Commits", "commits"),
        ("Active human contributors", "active_human_contributors"),
        ("New human contributors", "new_human_contributors"),
    ] {
        let values: Vec<String> = summaries.iter().map(|s| number(&s[key])).collect();
        writeln!(out, "| {label} | {} |", values.join(" | "))?;
    }

    let week = query("/api/dashboard?days=7")?;
    let active: Vec<&Value> = week["repository_activity"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["pull_requests_merged"].as_i64().unwrap_or(0) + r["commits"].as_i64().unwrap_or(0) > 0)
        .take(5)
        .collect();
    if !active.is_empty() {
        writeln!(out, "\n**Most active repositories, last 7 days**\n")?;
        writeln!(out, "| Repository | Merged PRs | Commits | Opened PRs |")?;
        writeln!(out, "|---|--:|--:|--:|")?;
        for row in active {
            writeln!(
                out,
                "| {} | {} | {} | {} |",
                row["name"].as_str().unwrap_or(""),
                number(&row["pull_requests_merged"]),
                number(&row["commits"]),
                number(&row["pull_requests_opened"]),
            )?;
        }
    }

    let ai = query("/api/ai-usage")?;
    let counts = &meta["counts"];
    writeln!(
        out,
        "\nThe snapshot holds {} repositories, {} contributors, {} pull requests, and {} counted commits; {} commits carry AI attribution.",
        number(&counts["repositories"]),
        number(&counts["contributors"]),
        number(&counts["pull_requests"]),
        number(&counts["commits"]),
        number(&ai["summary"]["commits_with_ai"]),
    )?;
    Ok(out)
}

pub fn today() -> i32 {
    let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    day_of(seconds as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(thousands(-1200), "-1,200");
        assert_eq!(duration(5), "5s");
        assert_eq!(duration(185), "3m 5s");
        assert_eq!(duration(3720), "1h 2m");
        assert_eq!(bytes(2_930_348), "2.8 MB");
        assert_eq!(utc("2026-10-18T22:44:34Z"), "2026-10-18 22:44 UTC");
    }
}
