//! Read and write the collected state in two interchangeable formats:
//!
//! - a single JSON document, gzipped when the path ends with `.gz` (the release asset);
//! - a directory of sorted JSON-lines files, one per table and, for the large tables,
//!   one per repository (committed to the `data` branch so daily diffs stay small).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use pulse_core::state::{STATE_SCHEMA_VERSION, State, Watermark};
use pulse_core::time::Ts;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const MANIFEST: &str = "manifest.json";

/// Whether a state path names a directory (no `.json` / `.gz` extension).
pub fn is_directory_path(path: &Path) -> bool {
    path.is_dir() || !matches!(path.extension().and_then(|e| e.to_str()), Some("gz" | "json"))
}

/// Read state from a JSON file (gzip or plain) or a JSON-lines directory.
pub fn read_state(path: &Path) -> Result<State> {
    let state = if path.is_dir() { read_state_dir(path)? } else { read_state_file(path)? };
    if state.schema_version != STATE_SCHEMA_VERSION {
        bail!(
            "unsupported state schema version {} in {} (expected {STATE_SCHEMA_VERSION})",
            state.schema_version,
            path.display()
        );
    }
    Ok(state)
}

/// Read state, or start empty when it does not exist yet.
pub fn read_state_or_default(path: &Path) -> Result<State> {
    let exists = if path.is_dir() { path.join(MANIFEST).exists() } else { path.exists() };
    if exists { read_state(path) } else { Ok(State::new()) }
}

/// Write state atomically in the format implied by the path.
pub fn write_state(path: &Path, state: &State) -> Result<()> {
    let mut state = state.clone();
    state.schema_version = STATE_SCHEMA_VERSION;
    state.normalize();
    if is_directory_path(path) { write_state_dir(path, &state) } else { write_state_file(path, &state) }
}

fn read_state_file(path: &Path) -> Result<State> {
    let json = read_gzip_or_plain(path).with_context(|| format!("cannot read state file {}", path.display()))?;
    serde_json::from_slice(&json).with_context(|| format!("invalid state file {}", path.display()))
}

fn write_state_file(path: &Path, state: &State) -> Result<()> {
    let directory = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(directory)?;
    let temporary = tempfile::NamedTempFile::new_in(directory)?;
    {
        let writer = BufWriter::new(temporary.as_file());
        if path.extension().is_some_and(|e| e == "gz") {
            let mut encoder = GzEncoder::new(writer, Compression::default());
            serde_json::to_writer(&mut encoder, state)?;
            encoder.finish()?.flush()?;
        } else {
            let mut writer = writer;
            serde_json::to_writer_pretty(&mut writer, state)?;
            writer.flush()?;
        }
    }
    let file = temporary.persist(path).with_context(|| format!("cannot write state file {}", path.display()))?;
    set_readable(&file)?;
    Ok(())
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Manifest {
    schema_version: u32,
    generated_at: Option<Ts>,
    catalog_revision: String,
    watermarks: Vec<Watermark>,
    /// Table name to row count, for a quick overview of the branch.
    rows: BTreeMap<String, usize>,
}

/// File name for a repository's partition: `<owner>/<name>.jsonl`.
fn partition_names(state: &State) -> BTreeMap<String, PathBuf> {
    state
        .repositories
        .iter()
        .map(|r| {
            let safe = |s: &str| s.replace(['/', '\\', ':'], "_");
            (r.repository_id.clone(), PathBuf::from(safe(&r.owner)).join(format!("{}.jsonl", safe(&r.name))))
        })
        .collect()
}

fn write_lines<T: Serialize>(path: &Path, rows: impl IntoIterator<Item = T>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer = BufWriter::new(File::create(path)?);
    for row in rows {
        serde_json::to_writer(&mut writer, &row)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn write_partitioned<'a, T: Serialize + 'a>(
    root: &Path,
    table: &str,
    rows: impl IntoIterator<Item = &'a T>,
    repository_id: impl Fn(&T) -> &str,
    partitions: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    let mut grouped: BTreeMap<PathBuf, Vec<&T>> = BTreeMap::new();
    for row in rows {
        let id = repository_id(row);
        let file = partitions
            .get(id)
            .cloned()
            .unwrap_or_else(|| PathBuf::from("_other").join(format!("{}.jsonl", id.replace(['/', '\\', ':'], "_"))));
        grouped.entry(file).or_default().push(row);
    }
    for (file, rows) in grouped {
        write_lines(&root.join(table).join(file), rows)?;
    }
    Ok(())
}

fn write_state_dir(path: &Path, state: &State) -> Result<()> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let staging = tempfile::Builder::new().prefix(".state-").tempdir_in(parent)?;
    let root = staging.path();
    let partitions = partition_names(state);
    write_lines(&root.join("repositories.jsonl"), &state.repositories)?;
    write_lines(&root.join("contributors.jsonl"), &state.contributors)?;
    write_lines(&root.join("email_identities.jsonl"), &state.email_identities)?;
    write_partitioned(root, "pull_requests", &state.pull_requests, |r| &r.repository_id, &partitions)?;
    write_partitioned(root, "commits", &state.commits, |r| &r.repository_id, &partitions)?;
    write_partitioned(root, "pull_request_commits", &state.pull_request_commits, |r| &r.repository_id, &partitions)?;
    let manifest = Manifest {
        schema_version: state.schema_version,
        generated_at: state.generated_at,
        catalog_revision: state.catalog_revision.clone(),
        watermarks: state.watermarks.clone(),
        rows: BTreeMap::from([
            ("repositories".into(), state.repositories.len()),
            ("contributors".into(), state.contributors.len()),
            ("email_identities".into(), state.email_identities.len()),
            ("pull_requests".into(), state.pull_requests.len()),
            ("commits".into(), state.commits.len()),
            ("pull_request_commits".into(), state.pull_request_commits.len()),
        ]),
    };
    let mut manifest_json = serde_json::to_vec_pretty(&manifest)?;
    manifest_json.push(b'\n');
    std::fs::write(root.join(MANIFEST), manifest_json)?;
    if path.exists() {
        // Only ever replace a previous state directory (or an empty one).
        let empty = path.is_dir() && std::fs::read_dir(path)?.next().is_none();
        if !path.join(MANIFEST).is_file() && !empty {
            bail!("refusing to replace {}: it is not a state directory (no {MANIFEST})", path.display());
        }
        std::fs::remove_dir_all(path).with_context(|| format!("cannot replace {}", path.display()))?;
    }
    let staged = staging.keep();
    std::fs::rename(&staged, path).with_context(|| format!("cannot write state directory {}", path.display()))?;
    set_readable_tree(path)?;
    Ok(())
}

fn read_lines<T: DeserializeOwned>(path: &Path, out: &mut Vec<T>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    for (number, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        out.push(
            serde_json::from_str(&line).with_context(|| format!("invalid row at {}:{}", path.display(), number + 1))?,
        );
    }
    Ok(())
}

fn read_partitioned<T: DeserializeOwned>(root: &Path, out: &mut Vec<T>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "jsonl") {
                files.push(path);
            }
        }
    }
    files.sort();
    for file in files {
        read_lines(&file, out)?;
    }
    Ok(())
}

fn read_state_dir(path: &Path) -> Result<State> {
    let manifest: Manifest = serde_json::from_slice(
        &std::fs::read(path.join(MANIFEST)).with_context(|| format!("{} has no {MANIFEST}", path.display()))?,
    )?;
    let mut state = State {
        schema_version: manifest.schema_version,
        generated_at: manifest.generated_at,
        catalog_revision: manifest.catalog_revision,
        watermarks: manifest.watermarks,
        ..State::default()
    };
    read_lines(&path.join("repositories.jsonl"), &mut state.repositories)?;
    read_lines(&path.join("contributors.jsonl"), &mut state.contributors)?;
    read_lines(&path.join("email_identities.jsonl"), &mut state.email_identities)?;
    read_partitioned(&path.join("pull_requests"), &mut state.pull_requests)?;
    read_partitioned(&path.join("commits"), &mut state.commits)?;
    read_partitioned(&path.join("pull_request_commits"), &mut state.pull_request_commits)?;
    state.normalize();
    Ok(state)
}

#[cfg(unix)]
fn set_readable(file: &File) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o644))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_readable(_file: &File) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_readable_tree(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if path.is_dir() { 0o755 } else { 0o644 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            set_readable_tree(&entry?.path())?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_readable_tree(_path: &Path) -> Result<()> {
    Ok(())
}

pub fn read_gzip_or_plain(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    BufReader::new(File::open(path)?).read_to_end(&mut bytes)?;
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut decoded = Vec::new();
        GzDecoder::new(bytes.as_slice()).read_to_end(&mut decoded)?;
        return Ok(decoded);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_core::state::{Commit, PullRequest, Repository};

    fn sample() -> State {
        let mut state = State::new();
        state.catalog_revision = "abc".into();
        state.repositories.push(Repository {
            repository_id: "github.com:R1".into(),
            owner: "flyteorg".into(),
            name: "flyte".into(),
            ..Repository::default()
        });
        state.pull_requests.push(PullRequest {
            repository_id: "github.com:R1".into(),
            provider_id: "7".into(),
            ..PullRequest::default()
        });
        state.commits.push(Commit {
            repository_id: "github.com:R1".into(),
            sha: "a".repeat(40),
            message: "Line one\n\nCo-authored-by: x".into(),
            ..Commit::default()
        });
        state.watermarks.push(Watermark { repository_id: "github.com:R1".into(), ..Watermark::default() });
        state
    }

    #[test]
    fn round_trips_both_formats() {
        let dir = tempfile::tempdir().unwrap();
        for path in [dir.path().join("state.json.gz"), dir.path().join("state")] {
            write_state(&path, &sample()).unwrap();
            let state = read_state(&path).unwrap();
            assert_eq!(state.catalog_revision, "abc");
            assert_eq!(state.commits[0].message, "Line one\n\nCo-authored-by: x");
            assert_eq!(state.pull_requests.len(), 1);
            assert_eq!(state.watermarks.len(), 1);
        }
        assert!(dir.path().join("state/commits/flyteorg/flyte.jsonl").exists());
        // Rewriting replaces the directory, dropping stale partitions.
        write_state(&dir.path().join("state"), &State::new()).unwrap();
        assert!(!dir.path().join("state/commits").exists());
        assert!(read_state_or_default(&dir.path().join("missing")).unwrap().commits.is_empty());
        // Never replace a directory that is not a state directory.
        std::fs::write(dir.path().join("keep.txt"), "x").unwrap();
        assert!(write_state(dir.path(), &State::new()).is_err());
        assert!(dir.path().join("keep.txt").exists());
    }
}
