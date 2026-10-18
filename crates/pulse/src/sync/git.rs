//! Ephemeral bare clones and the Git history queries used by the collector.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, bail};
use pulse_core::time::parse_timestamp;

/// Refspecs per `git fetch` invocation, keeping command lines well below OS limits.
const REFSPEC_BATCH: usize = 200;

/// `%x1e` starts a record, `%x1f` separates fields, and `%x1d` ends the raw body
/// so the `--numstat` lines that follow can be told apart from the message.
pub const LOG_FORMAT: &str = "--format=%x1e%H%x1f%P%x1f%an%x1f%ae%x1f%aI%x1f%cn%x1f%ce%x1f%cI%x1f%B%x1d";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitCommit {
    pub sha: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: Option<i64>,
    pub committer_name: String,
    pub committer_email: String,
    pub committed_at: Option<i64>,
    pub message: String,
    pub additions: i64,
    pub deletions: i64,
}

/// Parse `git log` output produced with [`LOG_FORMAT`] and optionally `--numstat`.
pub fn parse_log(output: &str) -> Vec<GitCommit> {
    let mut commits = Vec::new();
    for record in output.split('\x1e') {
        let Some((header, stats)) = record.split_once('\x1d') else {
            continue;
        };
        let fields: Vec<&str> = header.splitn(9, '\x1f').collect();
        if fields.len() != 9 || fields[0].is_empty() {
            continue;
        }
        let (mut additions, mut deletions) = (0, 0);
        for line in stats.lines() {
            let mut parts = line.splitn(3, '\t');
            let (Some(added), Some(removed), Some(_path)) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            // Binary files report "-" for both counts.
            additions += added.trim().parse::<i64>().unwrap_or(0);
            deletions += removed.trim().parse::<i64>().unwrap_or(0);
        }
        commits.push(GitCommit {
            sha: fields[0].trim().to_string(),
            parents: fields[1].split_whitespace().map(str::to_string).collect(),
            author_name: fields[2].to_string(),
            author_email: fields[3].to_string(),
            authored_at: parse_timestamp(fields[4]),
            committer_name: fields[5].to_string(),
            committer_email: fields[6].to_string(),
            committed_at: parse_timestamp(fields[7]),
            message: fields[8].trim_end_matches(['\n', '\r']).to_string(),
            additions,
            deletions,
        });
    }
    commits
}

/// Parse `<sha> <ref>` or `<sha>\t<ref>` lines for `refs/pull/<N>/head`.
pub fn parse_pull_heads(output: &str) -> HashMap<u64, String> {
    let mut heads = HashMap::new();
    for line in output.lines() {
        let mut parts = line.split_whitespace();
        let (Some(sha), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        let number = name
            .strip_prefix("refs/pull/")
            .and_then(|rest| rest.strip_suffix("/head"))
            .and_then(|n| n.parse::<u64>().ok());
        if let Some(number) = number {
            heads.insert(number, sha.to_string());
        }
    }
    heads
}

pub fn branch_refspec(branch: &str) -> String {
    format!("+refs/heads/{branch}:refs/heads/{branch}")
}

pub fn pull_refspec(number: u64) -> String {
    format!("+refs/pull/{number}/head:refs/pull/{number}/head")
}

pub struct GitRepo {
    pub dir: PathBuf,
}

impl GitRepo {
    /// Create an empty bare repository with `origin` pointing at `url`.
    pub fn init(dir: &Path, url: &str) -> Result<Self> {
        if dir.exists() {
            std::fs::remove_dir_all(dir).with_context(|| format!("cannot remove stale clone {}", dir.display()))?;
        }
        std::fs::create_dir_all(dir)?;
        let repo = Self { dir: dir.to_path_buf() };
        repo.run(&["init", "--quiet", "--bare"])?;
        repo.run(&["remote", "add", "origin", url])?;
        Ok(repo)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new("git");
        command
            .arg("--git-dir")
            .arg(&self.dir)
            .args(["-c", "gc.auto=0", "-c", "maintenance.auto=false", "-c", "core.quotePath=false"])
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C");
        command
    }

    pub fn run(&self, args: &[&str]) -> Result<String> {
        let output =
            self.command(args).stdin(Stdio::null()).output().with_context(|| "cannot run git; is it installed?")?;
        finish(args, output)
    }

    pub fn run_with_input(&self, args: &[&str], input: String) -> Result<String> {
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| "cannot run git; is it installed?")?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        // Write from another thread so a full stdout pipe cannot deadlock us.
        let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
        let output = child.wait_with_output()?;
        writer.join().expect("stdin writer panicked")?;
        finish(args, output)
    }

    /// Fetch refspecs in batches; `options` (for example `--shallow-since=...`)
    /// apply to every batch.
    pub fn fetch(&self, options: &[String], refspecs: &[String]) -> Result<()> {
        for batch in refspecs.chunks(REFSPEC_BATCH) {
            let mut args: Vec<&str> = vec!["fetch", "--quiet", "--no-tags", "--no-write-fetch-head"];
            args.extend(options.iter().map(String::as_str));
            args.push("origin");
            args.extend(batch.iter().map(String::as_str));
            self.run(&args)?;
        }
        Ok(())
    }

    /// Deepen (`--deepen=N`) or complete (`--unshallow`) the history of `refspecs`.
    pub fn deepen(&self, option: &str, refspecs: &[String]) -> Result<()> {
        for batch in refspecs.chunks(REFSPEC_BATCH) {
            if !self.is_shallow() {
                // Nothing left to deepen; a plain fetch keeps the refs current.
                self.fetch(&[], batch)?;
                continue;
            }
            self.fetch(&[option.to_string()], batch)?;
        }
        Ok(())
    }

    pub fn is_shallow(&self) -> bool {
        self.dir.join("shallow").exists()
    }

    /// Commits at the shallow boundary: present locally, but without their parents.
    pub fn shallow_commits(&self) -> HashSet<String> {
        std::fs::read_to_string(self.dir.join("shallow"))
            .map(|text| text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
            .unwrap_or_default()
    }

    pub fn remote_pull_heads(&self) -> Result<HashMap<u64, String>> {
        Ok(parse_pull_heads(&self.run(&["ls-remote", "origin", "refs/pull/*/head"])?))
    }

    pub fn local_pull_heads(&self) -> Result<HashMap<u64, String>> {
        Ok(parse_pull_heads(&self.run(&["for-each-ref", "--format=%(objectname) %(refname)", "refs/pull"])?))
    }

    pub fn rev_list(&self, args: &[String]) -> Result<Vec<String>> {
        let mut command: Vec<&str> = vec!["rev-list"];
        command.extend(args.iter().map(String::as_str));
        Ok(self.run(&command)?.lines().map(str::to_string).filter(|l| !l.is_empty()).collect())
    }

    /// Run many `git rev-list` queries on a few worker threads.
    pub fn rev_list_many(&self, jobs: &[Vec<String>]) -> Vec<Result<Vec<String>>> {
        let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
        let next = AtomicUsize::new(0);
        let results: Vec<Mutex<Option<Result<Vec<String>>>>> = jobs.iter().map(|_| Mutex::new(None)).collect();
        std::thread::scope(|scope| {
            for _ in 0..workers.min(jobs.len().max(1)) {
                scope.spawn(|| {
                    loop {
                        let index = next.fetch_add(1, Ordering::SeqCst);
                        if index >= jobs.len() {
                            break;
                        }
                        let result = self.rev_list(&jobs[index]);
                        *results[index].lock().expect("result lock") = Some(result);
                    }
                });
            }
        });
        results.into_iter().map(|slot| slot.into_inner().expect("result lock").expect("every job ran")).collect()
    }

    /// Commits reachable from `refs`, newest first.
    pub fn log(&self, refs: &[String]) -> Result<Vec<GitCommit>> {
        if refs.is_empty() {
            return Ok(Vec::new());
        }
        let mut args: Vec<&str> = vec!["log", "--numstat", LOG_FORMAT];
        args.extend(refs.iter().map(String::as_str));
        Ok(parse_log(&self.run(&args)?))
    }

    /// Details for exactly these commits.
    pub fn show_commits(&self, shas: &[String]) -> Result<Vec<GitCommit>> {
        if shas.is_empty() {
            return Ok(Vec::new());
        }
        let input = shas.join("\n") + "\n";
        Ok(parse_log(&self.run_with_input(&["log", "--no-walk=unsorted", "--stdin", "--numstat", LOG_FORMAT], input)?))
    }

    /// `second parent -> (merge commit, first parent)` for merges on the first-parent
    /// history of `branch_ref`, used to find merge-commit pull requests.
    pub fn first_parent_merges(&self, branch_ref: &str) -> Result<HashMap<String, (String, String)>> {
        let output = self.run(&["log", "--first-parent", "--merges", "--format=%H %P", branch_ref])?;
        let mut merges = HashMap::new();
        for line in output.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() >= 3 {
                merges.insert(fields[2].to_string(), (fields[0].to_string(), fields[1].to_string()));
            }
        }
        Ok(merges)
    }
}

fn finish(args: &[&str], output: std::process::Output) -> Result<String> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        let detail = &detail[detail.len().saturating_sub(1500)..];
        let shown: Vec<&str> = args.iter().take(4).copied().collect();
        bail!("git {} failed ({}): {detail}", shown.join(" "), output.status);
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_log_records_with_numstat() {
        let output = concat!(
            "\x1eaaa\x1fppp qqq\x1fAda\x1fada@example.com\x1f2025-08-19T10:31:52-07:00",
            "\x1fGitHub\x1fnoreply@github.com\x1f2025-08-19T17:31:52+00:00",
            "\x1fMerge pull request #1\n\nBody\x1fwith separator\n\x1d\n",
            "\x1ebbb\x1fccc\x1fBo\x1f1+bo@users.noreply.github.com\x1f2025-01-01T00:00:00Z",
            "\x1fBo\x1fbo@example.com\x1f2025-01-01T00:00:00Z\x1fFix (#2)\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n\x1d\n",
            "3\t1\tsrc/a.rs\n-\t-\timage.png\n10\t0\tsrc/b.rs\n",
        );
        let commits = parse_log(output);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].parents, vec!["ppp", "qqq"]);
        assert_eq!(commits[0].authored_at, commits[0].committed_at);
        assert_eq!(commits[0].message, "Merge pull request #1\n\nBody\x1fwith separator");
        assert_eq!((commits[0].additions, commits[0].deletions), (0, 0));
        assert_eq!(commits[1].message, "Fix (#2)\n\nCo-Authored-By: Claude <noreply@anthropic.com>");
        assert_eq!((commits[1].additions, commits[1].deletions), (13, 1));
        assert_eq!(commits[1].author_email, "1+bo@users.noreply.github.com");
    }

    #[test]
    fn parses_pull_heads() {
        let heads = parse_pull_heads("abc\trefs/pull/12/head\ndef refs/pull/7/head\nxyz refs/heads/main\n");
        assert_eq!(heads.get(&12).map(String::as_str), Some("abc"));
        assert_eq!(heads.get(&7).map(String::as_str), Some("def"));
        assert_eq!(heads.len(), 2);
    }
}
