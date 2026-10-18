//! Repository sync: commits from Git clones, pull requests from GitHub GraphQL.
//!
//! Git supplies commit details, branch membership, and pull request commit lists
//! (from `refs/pull/<N>/head`), so the GitHub API is only used for pull request
//! metadata and for mapping commit emails to accounts. A repository without a Git
//! cursor gets a full clone; later runs fetch shallowly from the cursor.

mod git;
mod github;
mod identity;
mod links;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use pulse_core::state::{Commit, EmailIdentity, PullRequest, PullRequestCommit, Repository, State};
use pulse_core::time::{Ts, format_timestamp, parse_timestamp};

use crate::config::{Catalog, RepositorySpec, load_catalog};
use crate::state_io::{read_state_or_default, write_state};
use git::{GitCommit, GitRepo, branch_refspec, pull_refspec};
use github::{GitHub, PullRequestNode, WaitBudgetExhausted};
use identity::{Account, Noreply, apply_identity_rules, apply_profile, email_hash, parse_noreply, upsert_contributor};
use links::{LinkInputs, plan_links, rev_list_args};

const DAY: i64 = 86_400;
/// Re-read pull requests updated shortly before the previous scan started.
const PR_OVERLAP_SECONDS: i64 = 600;
const PROFILE_REFRESH_SECONDS: i64 = 90 * DAY;
/// Emails without a GitHub account are looked up again after this long.
const EMAIL_RECHECK_SECONDS: i64 = 30 * DAY;
/// Total time one sync may spend waiting for GitHub rate limits.
const MAX_RATE_LIMIT_WAIT: Duration = Duration::from_secs(60 * 60);
const PR_PAGE_SIZE: usize = 100;
const DEEPEN_STEPS: [&str; 3] = ["--deepen=500", "--deepen=2000", "--unshallow"];
/// GitHub lists at most the 250 newest commits of a pull request. Keeping the same
/// limit stops history-import pull requests (thousands of commits from other
/// repositories) from flooding the commit counts.
const PR_COMMIT_LIMIT: usize = 250;

#[derive(clap::Args, Debug)]
pub struct SyncArgs {
    /// State file to read; a missing file starts an empty state.
    #[arg(long, default_value = "pulse-state.json.gz")]
    pub state: PathBuf,
    /// Where to write the updated state (defaults to --state).
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Directory containing repositories.yaml.
    #[arg(long, default_value = "config")]
    pub config_dir: PathBuf,
    /// Pull request history for repositories without a scan cursor; 0 scans all history.
    #[arg(long, default_value_t = 0)]
    pub history_days: u32,
    /// Discard the saved data for OWNER/NAME and collect it again (repeatable).
    #[arg(long, value_name = "OWNER/NAME")]
    pub reseed: Vec<String>,
    /// Directory for temporary clones (default: a new temporary directory).
    #[arg(long)]
    pub work_dir: Option<PathBuf>,
    /// Days of history before the Git cursor included in shallow fetches.
    #[arg(long, default_value_t = 7)]
    pub overlap_days: u32,
    /// Only sync OWNER/NAME (repeatable).
    #[arg(long, value_name = "OWNER/NAME")]
    pub only: Vec<String>,
}

#[derive(Debug, Default)]
struct RepoSummary {
    name: String,
    mode: &'static str,
    prs_scanned: usize,
    commits_from_git: usize,
    prs_linked: usize,
    link_rows: usize,
    link_failures: usize,
    new_contributors: usize,
    emails_looked_up: usize,
    api_requests: u64,
    api_points: u64,
}

struct RepoContext<'a> {
    spec: &'a RepositorySpec,
    catalog: &'a Catalog,
    args: &'a SyncArgs,
    work_root: &'a Path,
    reseed: bool,
    started: Ts,
}

fn now() -> Ts {
    Ts(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0))
}

fn github_token() -> Result<String> {
    ["PULSE_GITHUB_TOKEN", "GITHUB_TOKEN"]
        .iter()
        .find_map(|name| std::env::var(name).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()))
        .context("set PULSE_GITHUB_TOKEN or GITHUB_TOKEN to a GitHub token that can read public repositories")
}

/// Validate OWNER/NAME arguments against the active catalog; returns lowercased names.
fn resolve_names(catalog: &Catalog, names: &[String], flag: &str) -> Result<HashSet<String>> {
    let active: HashSet<String> =
        catalog.repositories.iter().filter(|r| r.is_active()).map(|r| r.full_name().to_lowercase()).collect();
    let mut selected = HashSet::new();
    for name in names {
        let key = name.trim().to_lowercase();
        if key.is_empty() {
            continue;
        }
        if !active.contains(&key) {
            bail!("{flag} {name:?}: repository is not active in the catalog");
        }
        selected.insert(key);
    }
    Ok(selected)
}

/// Removes a temporary clone when the repository sync finishes or fails.
struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn run(args: SyncArgs) -> Result<()> {
    let catalog = load_catalog(&args.config_dir.join("repositories.yaml"))?;
    let out = args.out.clone().unwrap_or_else(|| args.state.clone());
    let mut state = read_state_or_default(&args.state)?;
    let reseed = resolve_names(&catalog, &args.reseed, "--reseed")?;
    let only = resolve_names(&catalog, &args.only, "--only")?;
    let mut github = GitHub::new(github_token()?, MAX_RATE_LIMIT_WAIT)?;

    let mut temporary = None;
    let work_root = match &args.work_dir {
        Some(dir) => {
            std::fs::create_dir_all(dir)?;
            dir.clone()
        }
        None => {
            let dir = tempfile::Builder::new().prefix("pulse-sync-").tempdir()?;
            let path = dir.path().to_path_buf();
            temporary = Some(dir);
            path
        }
    };

    apply_identity_rules(&mut state.contributors, &catalog.identity_rules);
    let repositories: Vec<&RepositorySpec> = catalog
        .repositories
        .iter()
        .filter(|r| r.is_active() && (only.is_empty() || only.contains(&r.full_name().to_lowercase())))
        .collect();

    let mut summaries = Vec::new();
    let (mut failed, mut skipped) = (Vec::new(), Vec::new());
    let mut stopped = false;
    for spec in repositories {
        if stopped {
            skipped.push(spec.full_name());
            continue;
        }
        let context = RepoContext {
            spec,
            catalog: &catalog,
            args: &args,
            work_root: &work_root,
            reseed: reseed.contains(&spec.full_name().to_lowercase()),
            started: now(),
        };
        eprintln!("==> {}", spec.full_name());
        let backup = state.clone();
        match sync_repository(&mut state, &mut github, &context) {
            Ok(summary) => {
                summaries.push(summary);
                state.generated_at = Some(context.started);
                state.catalog_revision = catalog.revision.clone();
                // Save progress so a later failure or timeout keeps finished repositories.
                write_state(&out, &state)?;
            }
            Err(error) => {
                state = backup;
                if error.downcast_ref::<WaitBudgetExhausted>().is_some() {
                    println!("::warning::Stopping before finishing {}: {error:#}", spec.full_name());
                    stopped = true;
                    skipped.push(spec.full_name());
                } else {
                    println!("::warning::Sync failed for {}: {error:#}", spec.full_name());
                    failed.push(spec.full_name());
                }
            }
        }
    }

    if !stopped {
        match refresh_profiles(&mut state, &mut github, now()) {
            Ok(count) => eprintln!("==> profiles: refreshed {count}"),
            Err(error) => println!("::warning::Profile refresh incomplete: {error:#}"),
        }
    }
    apply_identity_rules(&mut state.contributors, &catalog.identity_rules);
    state.generated_at = Some(now());
    state.catalog_revision = catalog.revision.clone();
    write_state(&out, &state)?;
    drop(temporary);

    eprintln!("\nSync summary ({} GitHub requests, {} GraphQL points):", github.requests, github.points);
    for s in &summaries {
        eprintln!(
            "  {}: {} git, {} PRs scanned, {} commits from git, {} PRs linked ({} links, {} failures), {} new contributors, {} emails looked up, {} requests / {} points",
            s.name,
            s.mode,
            s.prs_scanned,
            s.commits_from_git,
            s.prs_linked,
            s.link_rows,
            s.link_failures,
            s.new_contributors,
            s.emails_looked_up,
            s.api_requests,
            s.api_points
        );
    }
    for name in &failed {
        eprintln!("  {name}: failed (previous data kept)");
    }
    for name in &skipped {
        eprintln!("  {name}: skipped (rate-limit budget exhausted)");
    }
    eprintln!(
        "State: {} repositories, {} contributors, {} pull requests, {} commits, {} links -> {}",
        state.repositories.len(),
        state.contributors.len(),
        state.pull_requests.len(),
        state.commits.len(),
        state.pull_request_commits.len(),
        out.display()
    );
    if summaries.is_empty() && !failed.is_empty() {
        bail!("every repository sync failed");
    }
    Ok(())
}

fn sync_repository(state: &mut State, github: &mut GitHub, ctx: &RepoContext<'_>) -> Result<RepoSummary> {
    let spec = ctx.spec;
    let key = spec.full_name().to_lowercase();
    let (requests_before, points_before) = (github.requests, github.points);
    let same_repo = |r: &Repository| format!("{}/{}", r.owner, r.name).to_lowercase() == key;

    let existing = state.repositories.iter().find(|r| same_repo(r)).cloned();
    let repository_id = match &existing {
        Some(row) if !row.repository_id.is_empty() => row.repository_id.clone(),
        _ if !spec.provider_id.is_empty() => format!("{}:{}", spec.host, spec.provider_id),
        _ => format!("{}:{}", spec.host, github.repository_node_id(&spec.owner, &spec.name)?),
    };
    let branch_changed = existing.as_ref().is_some_and(|row| row.target_branches != spec.target_branches);
    if ctx.reseed {
        eprintln!("  reseed: discarding saved pull requests, commits, links, and cursors");
        state.pull_requests.retain(|r| r.repository_id != repository_id);
        state.commits.retain(|r| r.repository_id != repository_id);
        state.pull_request_commits.retain(|r| r.repository_id != repository_id);
        state.watermarks.retain(|w| w.repository_id != repository_id);
    }
    state.repositories.retain(|r| r.repository_id != repository_id && !same_repo(r));
    state.repositories.push(Repository {
        repository_id: repository_id.clone(),
        provider_id: repository_id.split_once(':').map(|(_, id)| id.to_string()).unwrap_or_default(),
        host: spec.host.clone(),
        owner: spec.owner.clone(),
        name: spec.name.clone(),
        module: spec.module.clone(),
        status: spec.status.clone(),
        target_branches: spec.target_branches.clone(),
    });

    let watermark = state.watermark(&repository_id).cloned().unwrap_or_default();
    let initial_prs = ctx.reseed || branch_changed || watermark.pr_cursor.is_none();
    let full_git = ctx.reseed || branch_changed || watermark.git_cursor.is_none();
    let mut summary = RepoSummary {
        name: spec.full_name(),
        mode: if full_git { "full" } else { "shallow" },
        ..RepoSummary::default()
    };
    let contributors_before = state.contributors.len();

    // Pull requests from GraphQL.
    let since = if initial_prs {
        (ctx.args.history_days > 0).then(|| ctx.started.0 - i64::from(ctx.args.history_days) * DAY)
    } else {
        watermark.pr_cursor.map(|cursor| cursor.0 - PR_OVERLAP_SECONDS)
    };
    eprintln!(
        "  pull requests: {} scan{}",
        if initial_prs { "initial" } else { "incremental" },
        since.map(|s| format!(" since {}", format_timestamp(s))).unwrap_or_else(|| " of all history".into())
    );
    let scanned = scan_pull_requests(github, spec, since)?;
    summary.prs_scanned = scanned.len();

    let mut contributor_index: HashMap<String, usize> =
        state.contributors.iter().enumerate().map(|(i, c)| (c.id.clone(), i)).collect();
    let mut pr_index: HashMap<String, usize> = state
        .pull_requests
        .iter()
        .enumerate()
        .filter(|(_, pr)| pr.repository_id == repository_id)
        .map(|(i, pr)| (pr.provider_id.clone(), i))
        .collect();
    let linked: HashSet<String> = state
        .pull_request_commits
        .iter()
        .filter(|l| l.repository_id == repository_id)
        .map(|l| l.pull_request_id.clone())
        .collect();
    let mut needs_links = BTreeSet::new();
    for node in &scanned {
        let provider_id = node.provider_id();
        if provider_id.is_empty() {
            continue;
        }
        let author_id = match node.author_account() {
            Some(account) => {
                let row = account.to_contributor(ctx.started, &ctx.catalog.identity_rules);
                let id = row.id.clone();
                upsert_contributor(&mut state.contributors, &mut contributor_index, row);
                id
            }
            None => String::new(),
        };
        let row = pull_request_row(node, &repository_id, author_id, ctx.started);
        match pr_index.get(&provider_id) {
            Some(&index) => {
                if state.pull_requests[index].head_sha != row.head_sha || !linked.contains(&provider_id) {
                    needs_links.insert(provider_id.clone());
                }
                state.pull_requests[index] = row;
            }
            None => {
                needs_links.insert(provider_id.clone());
                pr_index.insert(provider_id, state.pull_requests.len());
                state.pull_requests.push(row);
            }
        }
    }

    // Git clone or shallow fetch.
    let clone_dir = ctx.work_root.join(format!("{}__{}.git", spec.owner, spec.name));
    let _cleanup = RemoveOnDrop(clone_dir.clone());
    let repo = GitRepo::init(&clone_dir, &format!("https://{}/{}/{}.git", spec.host, spec.owner, spec.name))?;
    let branches: Vec<String> = spec.target_branches.iter().map(|b| b.name.clone()).collect();
    let mut refspecs: Vec<String> = branches.iter().map(|b| branch_refspec(b)).collect();
    if full_git {
        refspecs.push("+refs/pull/*/head:refs/pull/*/head".into());
        eprintln!("  git: full clone of {} with pull request refs", branches.join(", "));
        repo.fetch(&[], &refspecs)?;
    } else {
        let remote_heads = repo.remote_pull_heads()?;
        let mut numbers = Vec::new();
        for provider_id in &needs_links {
            let pr = &state.pull_requests[pr_index[provider_id]];
            if remote_heads.contains_key(&pr.number) {
                numbers.push(pr.number);
            } else {
                eprintln!("  git: refs/pull/{}/head is not on GitHub; keeping its saved links", pr.number);
            }
        }
        numbers.sort_unstable();
        refspecs.extend(numbers.iter().map(|n| pull_refspec(*n)));
        let cursor = watermark.git_cursor.expect("shallow mode requires a Git cursor");
        let shallow_since = format_timestamp(cursor.0 - i64::from(ctx.args.overlap_days) * DAY);
        eprintln!(
            "  git: shallow fetch of {} and {} pull request refs since {shallow_since}",
            branches.join(", "),
            numbers.len()
        );
        if let Err(error) = repo.fetch(&[format!("--shallow-since={shallow_since}")], &refspecs) {
            eprintln!("  git: shallow fetch failed ({error:#}); retrying with --depth=1");
            repo.fetch(&["--depth=1".into()], &refspecs)?;
        }
    }

    let candidates: Vec<String> =
        if full_git { pr_index.keys().cloned().collect() } else { needs_links.into_iter().collect() };
    let mut step = 0;
    let analysis = loop {
        let analysis = analyze(&repo, &branches, state, &pr_index, &candidates)?;
        if !analysis.touches_boundary {
            break analysis;
        }
        if step >= DEEPEN_STEPS.len() || !repo.is_shallow() {
            eprintln!("  git: some pull request ranges still reach the shallow boundary");
            break analysis;
        }
        eprintln!("  git: pull request history reaches the shallow boundary; fetching with {}", DEEPEN_STEPS[step]);
        repo.deepen(DEEPEN_STEPS[step], &refspecs)?;
        step += 1;
    };

    // Details for linked commits that are not on a target branch.
    let mut known: HashSet<&str> = analysis.branch_commits.iter().map(|c| c.sha.as_str()).collect();
    let mut extra = Vec::new();
    for (_, shas) in &analysis.links {
        for sha in shas {
            if known.insert(sha.as_str()) {
                extra.push(sha.clone());
            }
        }
    }
    let linked_commits = repo.show_commits(&extra)?;
    let commits: Vec<&GitCommit> = analysis.branch_commits.iter().chain(linked_commits.iter()).collect();
    summary.commits_from_git = commits.len();
    upsert_commits(state, spec, &repository_id, &commits, &analysis, ctx.started);

    let recomputed: HashSet<&str> = analysis.links.iter().map(|(id, _)| id.as_str()).collect();
    state
        .pull_request_commits
        .retain(|l| !(l.repository_id == repository_id && recomputed.contains(l.pull_request_id.as_str())));
    for (provider_id, shas) in &analysis.links {
        for sha in shas {
            state.pull_request_commits.push(PullRequestCommit {
                repository_id: repository_id.clone(),
                pull_request_id: provider_id.clone(),
                commit_sha: sha.clone(),
            });
        }
        summary.link_rows += shas.len();
    }
    summary.prs_linked = analysis.links.len();
    summary.link_failures = analysis.failures;
    eprintln!(
        "  git: {} commits, {} pull requests linked ({} links, {} failures)",
        commits.len(),
        summary.prs_linked,
        summary.link_rows,
        summary.link_failures
    );

    summary.emails_looked_up = resolve_accounts(state, github, ctx, &repository_id, &commits, &mut contributor_index)?;

    let watermark = state.watermark_mut(&repository_id);
    watermark.pr_cursor = Some(ctx.started);
    watermark.git_cursor = Some(ctx.started);
    summary.new_contributors = state.contributors.len() - contributors_before;
    summary.api_requests = github.requests - requests_before;
    summary.api_points = github.points - points_before;
    Ok(summary)
}

fn scan_pull_requests(github: &mut GitHub, spec: &RepositorySpec, since: Option<i64>) -> Result<Vec<PullRequestNode>> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for branch in &spec.target_branches {
        let (mut after, mut page_size, mut pages, before) = (None::<String>, PR_PAGE_SIZE, 0, found.len());
        loop {
            let page =
                match github.pull_request_page(&spec.owner, &spec.name, &branch.name, after.as_deref(), page_size) {
                    Ok(page) => page,
                    Err(error) if page_size > 25 && error.downcast_ref::<WaitBudgetExhausted>().is_none() => {
                        page_size /= 2;
                        eprintln!("  pull requests: page failed ({error:#}); retrying with {page_size} per page");
                        continue;
                    }
                    Err(error) => return Err(error),
                };
            pages += 1;
            let mut reached_older = false;
            for node in page.nodes.into_iter().flatten() {
                let updated = node.updated_at.as_deref().and_then(parse_timestamp);
                if let (Some(since), Some(updated)) = (since, updated)
                    && updated < since
                {
                    reached_older = true;
                    continue;
                }
                if seen.insert(node.provider_id()) {
                    found.push(node);
                }
            }
            if reached_older || !page.page_info.has_next_page || page.page_info.end_cursor.is_none() {
                break;
            }
            after = page.page_info.end_cursor;
            if pages % 10 == 0 {
                eprintln!("  pull requests: {} on {} after {pages} pages", found.len() - before, branch.name);
            }
        }
        eprintln!("  pull requests: {} updated on {} ({pages} pages)", found.len() - before, branch.name);
    }
    Ok(found)
}

fn pull_request_row(node: &PullRequestNode, repository_id: &str, author_id: String, observed_at: Ts) -> PullRequest {
    let ts = |value: &Option<String>| value.as_deref().and_then(parse_timestamp).map(Ts);
    let merged_at = ts(&node.merged_at);
    PullRequest {
        repository_id: repository_id.to_string(),
        provider_id: node.provider_id(),
        number: node.number,
        author_contributor_id: author_id,
        title: node.title.clone(),
        state: if merged_at.is_some() { "merged".into() } else { node.state.to_lowercase() },
        created_at: ts(&node.created_at),
        updated_at: ts(&node.updated_at),
        closed_at: ts(&node.closed_at),
        merged_at,
        target_branch: node.base_ref_name.clone(),
        head_sha: node.head_ref_oid.clone().unwrap_or_default(),
        merge_commit_sha: node.merge_commit.as_ref().map(|m| m.oid.clone()).unwrap_or_default(),
        additions: node.additions.unwrap_or(0),
        deletions: node.deletions.unwrap_or(0),
        commit_count: node.commits.as_ref().map(|c| c.total_count).unwrap_or(0),
        url: node.url.clone(),
        observed_at: Some(observed_at),
    }
}

struct Analysis {
    branch_commits: Vec<GitCommit>,
    /// Commit SHA to the tracked branches whose history contains it.
    membership: HashMap<String, BTreeSet<String>>,
    /// Recomputed pull request ID to its commit SHAs.
    links: Vec<(String, Vec<String>)>,
    shallow: HashSet<String>,
    touches_boundary: bool,
    failures: usize,
}

fn analyze(
    repo: &GitRepo,
    branches: &[String],
    state: &State,
    pr_index: &HashMap<String, usize>,
    candidates: &[String],
) -> Result<Analysis> {
    let shallow = repo.shallow_commits();
    let local_heads = repo.local_pull_heads()?;
    let branch_refs: Vec<String> = branches.iter().map(|b| format!("refs/heads/{b}")).collect();
    let mut membership: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut merges = HashMap::new();
    for (branch, branch_ref) in branches.iter().zip(&branch_refs) {
        for sha in repo.rev_list(std::slice::from_ref(branch_ref))? {
            membership.entry(sha).or_default().insert(branch.clone());
        }
        merges.extend(repo.first_parent_merges(branch_ref)?);
    }
    let branch_commits = repo.log(&branch_refs)?;
    let parents: HashMap<&str, &Vec<String>> = branch_commits.iter().map(|c| (c.sha.as_str(), &c.parents)).collect();

    let mut plans = Vec::new();
    for provider_id in candidates {
        let pr = &state.pull_requests[pr_index[provider_id]];
        let inputs = LinkInputs {
            number: pr.number,
            head_sha: &pr.head_sha,
            merge_commit_sha: &pr.merge_commit_sha,
            pull_head: local_heads.get(&pr.number).map(String::as_str),
        };
        if let Some(plan) = plan_links(&inputs, |sha| parents.get(sha).map(|p| (*p).clone()), &merges) {
            // GitHub lists a pull request's commits relative to its own base branch.
            let base: Vec<String> =
                if branches.contains(&pr.target_branch) { vec![pr.target_branch.clone()] } else { branches.to_vec() };
            plans.push((provider_id.clone(), pr.number, rev_list_args(&plan, &base)));
        }
    }
    let jobs: Vec<Vec<String>> = plans.iter().map(|(_, _, args)| args.clone()).collect();
    let results = repo.rev_list_many(&jobs);

    let (mut links, mut touches_boundary, mut failures) = (Vec::new(), false, 0);
    for ((provider_id, number, _), result) in plans.into_iter().zip(results) {
        match result {
            Ok(mut shas) => {
                touches_boundary |= shas.iter().any(|sha| shallow.contains(sha));
                // rev-list prints newest first, like GitHub's list.
                shas.truncate(PR_COMMIT_LIMIT);
                links.push((provider_id, shas));
            }
            Err(error) => {
                // A missing object in a shallow clone means the history must be deepened.
                touches_boundary |= !shallow.is_empty();
                failures += 1;
                eprintln!("  git: cannot list commits for pull request #{number}: {error:#}");
            }
        }
    }
    Ok(Analysis { branch_commits, membership, links, shallow, touches_boundary, failures })
}

fn upsert_commits(
    state: &mut State,
    spec: &RepositorySpec,
    repository_id: &str,
    commits: &[&GitCommit],
    analysis: &Analysis,
    observed_at: Ts,
) {
    let mut index: HashMap<String, usize> = state
        .commits
        .iter()
        .enumerate()
        .filter(|(_, c)| c.repository_id == repository_id)
        .map(|(i, c)| (c.sha.clone(), i))
        .collect();
    for commit in commits {
        let position = *index.entry(commit.sha.clone()).or_insert_with(|| {
            state.commits.push(Commit {
                repository_id: repository_id.to_string(),
                sha: commit.sha.clone(),
                ..Commit::default()
            });
            state.commits.len() - 1
        });
        let boundary = analysis.shallow.contains(&commit.sha);
        let row = &mut state.commits[position];
        row.author_name = commit.author_name.clone();
        row.committer_name = commit.committer_name.clone();
        row.author_email_hash = email_hash(&commit.author_email);
        row.committer_email_hash = email_hash(&commit.committer_email);
        row.authored_at = commit.authored_at.map(Ts);
        row.committed_at = commit.committed_at.map(Ts);
        row.message = commit.message.clone();
        row.url = format!("https://github.com/{}/{}/commit/{}", spec.owner, spec.name, commit.sha);
        row.observed_at = Some(observed_at);
        row.from_git = true;
        if let Some(branches) = analysis.membership.get(&commit.sha) {
            let mut merged: BTreeSet<String> = row.branches.iter().cloned().collect();
            merged.extend(branches.iter().cloned());
            row.branches = merged.into_iter().collect();
        }
        // A shallow-boundary commit looks parentless, and its diff would be the whole tree.
        if !boundary {
            row.parent_shas = commit.parents.clone();
            if commit.parents.len() <= 1 {
                row.additions = commit.additions;
                row.deletions = commit.deletions;
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Role {
    Author,
    Committer,
}

enum Lookup {
    Account(String),
    NoAccount,
    Unknown,
}

/// Map commit author/committer emails to GitHub accounts. Returns the number of
/// emails looked up through the API.
fn resolve_accounts(
    state: &mut State,
    github: &mut GitHub,
    ctx: &RepoContext<'_>,
    repository_id: &str,
    commits: &[&GitCommit],
    contributor_index: &mut HashMap<String, usize>,
) -> Result<usize> {
    let rules = &ctx.catalog.identity_rules;
    let commit_index: HashMap<String, usize> = state
        .commits
        .iter()
        .enumerate()
        .filter(|(_, c)| c.repository_id == repository_id)
        .map(|(i, c)| (c.sha.clone(), i))
        .collect();
    let mut email_index: HashMap<String, usize> =
        state.email_identities.iter().enumerate().map(|(i, e)| (e.email_hash.clone(), i)).collect();
    let started = ctx.started;
    let roles = [Role::Author, Role::Committer];
    let email_of = |commit: &GitCommit, role: Role| match role {
        Role::Author => commit.author_email.clone(),
        Role::Committer => commit.committer_email.clone(),
    };
    let field = |row: &Commit, role: Role| match role {
        Role::Author => row.author_contributor_id.clone(),
        Role::Committer => row.committer_contributor_id.clone(),
    };

    // Learn from accounts already attached to commits (e.g. seeded REST data).
    for commit in commits {
        let row = &state.commits[commit_index[&commit.sha]];
        let pairs: Vec<(String, String)> =
            roles.iter().map(|&role| (email_hash(&email_of(commit, role)), field(row, role))).collect();
        for (hash, id) in pairs {
            if !hash.is_empty() && !id.is_empty() {
                remember_email(state, &mut email_index, &hash, &id, false, started);
            }
        }
    }

    let login_index: HashMap<String, String> = state
        .contributors
        .iter()
        .filter(|c| !c.login.is_empty())
        .map(|c| (c.login.to_lowercase(), c.id.clone()))
        .collect();
    let mut unknown: HashMap<String, (String, Role)> = HashMap::new();
    for commit in commits {
        for role in roles {
            let email = email_of(commit, role);
            let hash = email_hash(&email);
            if hash.is_empty() || !field(&state.commits[commit_index[&commit.sha]], role).is_empty() {
                continue;
            }
            let lookup = match parse_noreply(&email) {
                Some(Noreply::WithId { id, login }) => {
                    let account = Account {
                        database_id: id.parse().unwrap_or_default(),
                        account_type: if login.to_lowercase().ends_with("[bot]") {
                            "Bot".into()
                        } else {
                            "User".into()
                        },
                        login,
                        ..Account::default()
                    };
                    let id = account.contributor_id();
                    if !contributor_index.contains_key(&id) {
                        let mut row = account.to_contributor(ctx.started, rules);
                        row.profile_observed_at = None;
                        upsert_contributor(&mut state.contributors, contributor_index, row);
                    }
                    remember_email(state, &mut email_index, &hash, &id, false, started);
                    Lookup::Account(id)
                }
                Some(Noreply::LoginOnly { login }) if login_index.contains_key(&login.to_lowercase()) => {
                    Lookup::Account(login_index[&login.to_lowercase()].clone())
                }
                _ => match email_index.get(&hash).map(|&i| &state.email_identities[i]) {
                    Some(entry) if !entry.contributor_id.is_empty() => Lookup::Account(entry.contributor_id.clone()),
                    Some(entry) if entry.checked_at.is_some_and(|t| ctx.started.0 - t.0 < EMAIL_RECHECK_SECONDS) => {
                        Lookup::NoAccount
                    }
                    _ => Lookup::Unknown,
                },
            };
            match lookup {
                Lookup::Account(id) => set_commit_account(state, commit_index[&commit.sha], role, &id),
                Lookup::NoAccount => {}
                Lookup::Unknown => {
                    unknown.entry(hash).or_insert_with(|| (commit.sha.clone(), role));
                }
            }
        }
    }
    if unknown.is_empty() {
        return Ok(0);
    }

    let mut representatives: Vec<String> = unknown.values().map(|(sha, _)| sha.clone()).collect();
    representatives.sort();
    representatives.dedup();
    eprintln!("  accounts: looking up {} commit emails via {} commits", unknown.len(), representatives.len());
    let accounts = github.commit_accounts(&ctx.spec.owner, &ctx.spec.name, &representatives)?;
    for (hash, (sha, role)) in &unknown {
        let account = accounts.get(sha).and_then(|(author, committer)| match role {
            Role::Author => author.clone(),
            Role::Committer => committer.clone(),
        });
        let id = match account {
            Some(account) => {
                let row = account.to_contributor(ctx.started, rules);
                let id = row.id.clone();
                upsert_contributor(&mut state.contributors, contributor_index, row);
                id
            }
            None => String::new(),
        };
        remember_email(state, &mut email_index, hash, &id, true, started);
    }

    // Apply the new mappings to every commit seen in this run.
    for commit in commits {
        for role in roles {
            let position = commit_index[&commit.sha];
            if !field(&state.commits[position], role).is_empty() {
                continue;
            }
            let hash = email_hash(&email_of(commit, role));
            if let Some(&i) = email_index.get(&hash) {
                let id = state.email_identities[i].contributor_id.clone();
                if !id.is_empty() {
                    set_commit_account(state, position, role, &id);
                }
            }
        }
    }
    Ok(unknown.len())
}

/// Record `hash -> id`. Existing mappings change only when `overwrite` is set
/// (a fresh API answer) or when an account replaces a "no account" entry.
fn remember_email(
    state: &mut State,
    email_index: &mut HashMap<String, usize>,
    hash: &str,
    id: &str,
    overwrite: bool,
    at: Ts,
) {
    match email_index.get(hash) {
        Some(&i) => {
            let entry = &mut state.email_identities[i];
            if overwrite || (entry.contributor_id.is_empty() && !id.is_empty()) {
                entry.contributor_id = id.to_string();
                entry.checked_at = Some(at);
            }
        }
        None => {
            email_index.insert(hash.to_string(), state.email_identities.len());
            state.email_identities.push(EmailIdentity {
                email_hash: hash.to_string(),
                contributor_id: id.to_string(),
                checked_at: Some(at),
            });
        }
    }
}

fn set_commit_account(state: &mut State, position: usize, role: Role, id: &str) {
    let row = &mut state.commits[position];
    match role {
        Role::Author => row.author_contributor_id = id.to_string(),
        Role::Committer => row.committer_contributor_id = id.to_string(),
    }
}

fn valid_login(login: &str) -> bool {
    !login.is_empty() && login.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Refresh public profiles that were never checked or are older than 90 days.
fn refresh_profiles(state: &mut State, github: &mut GitHub, now: Ts) -> Result<usize> {
    let due: Vec<(usize, String)> = state
        .contributors
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            valid_login(c.login.trim())
                && (!c.profile_name_checked
                    || c.profile_observed_at.is_none_or(|t| now.0 - t.0 >= PROFILE_REFRESH_SECONDS))
        })
        .map(|(i, c)| (i, c.login.trim().to_string()))
        .collect();
    if due.is_empty() {
        return Ok(0);
    }
    eprintln!("==> profiles: looking up {} public GitHub profiles", due.len());
    let logins: Vec<String> = due.iter().map(|(_, login)| login.clone()).collect();
    let profiles = github.users(&logins)?;
    let mut updated = 0;
    for (position, login) in due {
        let Some(Some(profile)) = profiles.get(&login) else {
            continue;
        };
        let row = &mut state.contributors[position];
        // A login can move to a different account; never mix two identities.
        if !row.provider_user_id.is_empty() && row.provider_user_id != profile.database_id.to_string() {
            continue;
        }
        apply_profile(row, profile, now);
        updated += 1;
    }
    Ok(updated)
}
