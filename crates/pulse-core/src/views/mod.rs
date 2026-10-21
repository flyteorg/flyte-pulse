//! Dashboard queries over a loaded snapshot. The browser views call them with
//! HTTP-style API URLs and receive JSON responses.

mod activity;
mod ai;
mod company;
mod languages;
mod people;

use std::collections::HashMap;
use std::hash::Hash;

use serde_json::{Value, json};

use crate::snapshot::{CommitRecord, MetricKind, NONE, Person, Pr, Repo, Snapshot};
use crate::time::format_iso;

/// An HTTP-style error returned to the browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: u16,
    pub detail: String,
}

impl ApiError {
    pub fn new(status: u16, detail: impl Into<String>) -> Self {
        Self { status, detail: detail.into() }
    }
}

pub type ApiResult = Result<Value, ApiError>;

/// A contributor's identity kind, parsed once from the snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Human,
    Bot,
    Ai,
    Unknown,
}

impl Kind {
    fn parse(value: &str) -> Self {
        if value.eq_ignore_ascii_case("HUMAN") {
            Self::Human
        } else if value.eq_ignore_ascii_case("BOT") {
            Self::Bot
        } else if value.eq_ignore_ascii_case("AI") {
            Self::Ai
        } else {
            Self::Unknown
        }
    }
}

/// A snapshot plus lookup indexes built once at load time.
pub struct Dataset {
    pub snapshot: Snapshot,
    repo_by_id: HashMap<String, u32>,
    key_by_id: HashMap<String, u32>,
    /// Identity kind per contributor index.
    kinds: Vec<Kind>,
    metrics_by_repo: Vec<Vec<u32>>,
    /// Metric rows per contributor key index (contributors, then extra keys).
    metrics_by_key: Vec<Vec<u32>>,
    metric_dates: Option<(i32, i32)>,
    commits_by_pr: Vec<Vec<u32>>,
    /// Branch indices of the pull requests linking each commit, sorted by name.
    commit_branches: Vec<Vec<u32>>,
    /// Pull request indices, newest `created_at` first.
    prs_newest_first: Vec<u32>,
}

impl Dataset {
    pub fn new(snapshot: Snapshot) -> Self {
        let repo_by_id = snapshot.repositories.iter().enumerate().map(|(i, r)| (r.id.clone(), i as u32)).collect();
        let mut key_by_id: HashMap<String, u32> = HashMap::new();
        for (i, person) in snapshot.contributors.iter().enumerate() {
            key_by_id.entry(person.id.clone()).or_insert(i as u32);
        }
        let base = snapshot.contributors.len() as u32;
        for (i, key) in snapshot.extra_keys.iter().enumerate() {
            key_by_id.entry(key.clone()).or_insert(base + i as u32);
        }
        let kinds = snapshot.contributors.iter().map(|p| Kind::parse(&p.kind)).collect();
        let mut metrics_by_repo = vec![Vec::new(); snapshot.repositories.len()];
        let mut metrics_by_key = vec![Vec::new(); snapshot.contributors.len() + snapshot.extra_keys.len()];
        let mut metric_dates: Option<(i32, i32)> = None;
        for (i, row) in snapshot.metrics.iter().enumerate() {
            metrics_by_repo[row.repo as usize].push(i as u32);
            if let Some(rows) = metrics_by_key.get_mut(row.key as usize) {
                rows.push(i as u32);
            }
            metric_dates = Some(match metric_dates {
                None => (row.date, row.date),
                Some((first, last)) => (first.min(row.date), last.max(row.date)),
            });
        }
        let mut commits_by_pr = vec![Vec::new(); snapshot.pull_requests.len()];
        let mut commit_branches: Vec<Vec<u32>> = vec![Vec::new(); snapshot.commits.len()];
        for &(pr, commit) in &snapshot.pr_commits {
            commits_by_pr[pr as usize].push(commit);
            let branch = snapshot.pull_requests[pr as usize].branch;
            if branch != 0 && !commit_branches[commit as usize].contains(&branch) {
                commit_branches[commit as usize].push(branch);
            }
        }
        for branches in &mut commit_branches {
            branches.sort_by(|a, b| snapshot.branches[*a as usize].cmp(&snapshot.branches[*b as usize]));
        }
        let mut prs_newest_first: Vec<u32> = (0..snapshot.pull_requests.len() as u32).collect();
        // Stable sort, newest first; pull requests without a creation time go last.
        prs_newest_first.sort_by(|a, b| {
            let created = |i: &u32| snapshot.pull_requests[*i as usize].created_at.unwrap_or(i64::MIN);
            created(b).cmp(&created(a))
        });
        Self {
            snapshot,
            repo_by_id,
            key_by_id,
            kinds,
            metrics_by_repo,
            metrics_by_key,
            metric_dates,
            commits_by_pr,
            commit_branches,
            prs_newest_first,
        }
    }

    pub fn repo(&self, index: u32) -> &Repo {
        &self.snapshot.repositories[index as usize]
    }

    pub fn repo_index(&self, id: &str) -> Option<u32> {
        self.repo_by_id.get(id).copied()
    }

    pub fn key_index(&self, id: &str) -> Option<u32> {
        self.key_by_id.get(id).copied()
    }

    /// The identifier behind a contributor key index ("" for none).
    pub fn key_id(&self, key: u32) -> &str {
        if key == NONE {
            return "";
        }
        let contributors = &self.snapshot.contributors;
        match contributors.get(key as usize) {
            Some(person) => &person.id,
            None => self.snapshot.extra_keys.get(key as usize - contributors.len()).map_or("", String::as_str),
        }
    }

    pub fn person(&self, key: u32) -> Option<&Person> {
        if key == NONE { None } else { self.snapshot.contributors.get(key as usize) }
    }

    /// The identity kind of a contributor key; extra keys and `NONE` are unknown.
    pub fn kind(&self, key: u32) -> Kind {
        self.kinds.get(key as usize).copied().unwrap_or(Kind::Unknown)
    }

    pub fn is_human(&self, key: u32) -> bool {
        self.kind(key) == Kind::Human
    }

    pub fn is_bot(&self, key: u32) -> bool {
        self.kind(key) == Kind::Bot
    }

    pub fn is_ai(&self, key: u32) -> bool {
        self.kind(key) == Kind::Ai
    }

    /// A human outside the maintainer companies on `date`.
    pub fn is_community(&self, key: u32, date: i32) -> bool {
        self.is_human(key)
            && self
                .person(key)
                .is_some_and(|p| !p.maintainer_days.iter().any(|&(start, end)| start <= date && date < end))
    }

    pub fn branch(&self, index: u32) -> &str {
        self.snapshot.branches.get(index as usize).map_or("", String::as_str)
    }

    pub fn branch_index(&self, name: &str) -> Option<u32> {
        self.snapshot.branches.iter().position(|b| b == name).map(|i| i as u32)
    }

    /// The branch index to filter rows by: an unknown branch name matches no row.
    pub fn branch_filter(&self, name: &str) -> u32 {
        self.branch_index(name).unwrap_or(NONE - 1)
    }

    /// Repositories as `{id, name}` options, sorted by name.
    pub fn repository_options(&self) -> Vec<Value> {
        let mut repositories: Vec<&Repo> = self.snapshot.repositories.iter().collect();
        repositories.sort_by_cached_key(|r| fold(&r.full_name()));
        repositories.into_iter().map(|r| json!({"id": r.id, "name": r.full_name()})).collect()
    }

    /// Display name, then login, then the raw identifier.
    pub fn display(&self, key: u32) -> String {
        self.name_or(key, self.key_id(key))
    }

    pub fn login(&self, key: u32) -> String {
        self.person(key).map(|p| p.login.clone()).unwrap_or_default()
    }

    /// Display name or login of a contributor, falling back to a Git name.
    pub fn name_or(&self, key: u32, fallback: &str) -> String {
        match self.person(key) {
            Some(p) if !p.display_name.is_empty() => p.display_name.clone(),
            Some(p) if !p.login.is_empty() => p.login.clone(),
            _ => fallback.to_string(),
        }
    }

    pub fn commit_url(&self, commit: &CommitRecord) -> String {
        let repo = self.repo(commit.repo);
        format!("https://github.com/{}/{}/commit/{}", repo.owner, repo.name, commit.sha_hex())
    }

    /// The fields every commit list shows; views add their own on top.
    pub(crate) fn commit_view(&self, index: u32) -> Value {
        let commit = &self.snapshot.commits[index as usize];
        fn git_name(name: &str) -> &str {
            if name.is_empty() { "Unknown" } else { name }
        }
        json!({
            "sha": commit.sha_hex(),
            "repository_id": self.repo(commit.repo).id,
            "repository": self.repo(commit.repo).full_name(),
            "target_branches": self.commit_branches[index as usize].iter().map(|b| self.branch(*b)).collect::<Vec<_>>(),
            "committed_at": iso(commit.committed_at),
            "author_id": self.key_id(commit.author),
            "author_name": self.name_or(commit.author, git_name(&commit.author_name)),
            "author_login": self.login(commit.author),
            "committer_id": self.key_id(commit.committer),
            "committer_name": self.name_or(commit.committer, git_name(&commit.committer_name)),
            "committer_login": self.login(commit.committer),
            "url": self.commit_url(commit),
        })
    }

    pub fn pr_url(&self, pr: &Pr) -> String {
        let repo = self.repo(pr.repo);
        format!("https://github.com/{}/{}/pull/{}", repo.owner, repo.name, pr.number)
    }

    pub fn meta(&self) -> Value {
        let meta = &self.snapshot.meta;
        json!({
            "version": meta.version,
            "generated_at": format_iso(meta.generated_at),
            "raw_generated_at": format_iso(meta.raw_generated_at),
            "raw_version": meta.raw_version,
            "catalog_revision": meta.catalog_revision,
            "mapping_revision": meta.mapping_revision,
        })
    }

    /// Route an API URL such as `/api/dashboard?days=30`.
    /// `today` is the viewer's current UTC date as days since 1970-01-01.
    pub fn request(&self, url: &str, today: i32) -> ApiResult {
        let (path, query) = url.split_once('?').unwrap_or((url, ""));
        let params = Params::parse(query);
        let segments: Vec<String> =
            path.trim_matches('/').split('/').filter(|s| !s.is_empty()).map(percent_decode).collect();
        let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
        let view = View { data: self, today };
        match segments.as_slice() {
            ["api", "meta"] => Ok(self.meta()),
            ["api", "dashboard"] => {
                let days = parse_days(params.get_or("days", "30"))?;
                Ok(view.dashboard(activity::Scope::All, days, params.get("repository_id"), None))
            }
            ["api", "ai-dashboard"] => {
                let days = parse_days(params.get_or("days", "30"))?;
                let start = days.map(|d| today - (d - 1));
                let usage = view.ai_usage(&ai::AiQuery {
                    start,
                    end: Some(today),
                    repository_id: params.get("repository_id"),
                    top_limit: None,
                    ..ai::AiQuery::default()
                });
                Ok(json!({"ai_top_contributors": usage["top_ais"]}))
            }
            ["api", "repositories"] => Ok(json!({"repositories": view.repository_list()})),
            ["api", "repositories", id] => {
                let days = parse_days(params.get_or("days", "365"))?;
                view.repository_detail(id, days, params.get("target_branch"))
                    .ok_or_else(|| ApiError::new(404, "Repository not found"))
            }
            ["api", "users"] => Ok(view.contributor_index()),
            ["api", "users", id] | ["api", "users", id, "history"] => {
                let days = parse_days(params.get_or("days", "365"))?;
                let history = segments.len() == 4;
                view.contributor_detail(id, days, params.get("repository_id"), history)
                    .ok_or_else(|| ApiError::new(404, "Contributor not found"))
            }
            ["api", "companies"] => view.companies("", &params),
            ["api", "companies", id] => {
                let id = if *id == "unknown" { "independent" } else { id };
                view.companies(id, &params)
            }
            ["api", "ai-usage"] => Ok(view.ai_usage(&ai::AiQuery { top_limit: Some(20), ..ai::AiQuery::default() })),
            ["api", "ai-contributors", key] => {
                let days = parse_days(params.get_or("days", "365"))?;
                let usage = view.ai_usage(&ai::AiQuery {
                    start: days.map(|d| today - (d - 1)),
                    end: Some(today),
                    repository_id: params.get("repository_id"),
                    identity_key: Some(key.to_string()),
                    provider: params.get_or("provider", "").to_string(),
                    model: params.get_or("model", "").to_string(),
                    name: params.get_or("name", "").to_string(),
                    top_limit: Some(0),
                    ..ai::AiQuery::default()
                });
                Ok(usage["identity_detail"].clone())
            }
            _ => Err(ApiError::new(404, "Not Found")),
        }
    }
}

/// A request in flight: the dataset and the viewer's current date.
pub(crate) struct View<'a> {
    pub data: &'a Dataset,
    pub today: i32,
}

pub(crate) struct Params(Vec<(String, String)>);

impl Params {
    fn parse(query: &str) -> Self {
        Self(
            query
                .split('&')
                .filter(|pair| !pair.is_empty())
                .map(|pair| {
                    let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                    (percent_decode(&key.replace('+', " ")), percent_decode(&value.replace('+', " ")))
                })
                .collect(),
        )
    }

    /// A non-empty parameter value; `?x=` counts as absent.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty())
    }

    pub fn get_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.0.iter().rev().find(|(k, _)| k == key).map_or(default, |(_, v)| v.as_str())
    }
}

pub fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(byte) =
                std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_days(value: &str) -> Result<Option<i32>, ApiError> {
    match value {
        "7" | "30" | "90" | "365" => Ok(Some(value.parse().unwrap_or(30))),
        "all" => Ok(None),
        _ => Err(ApiError::new(422, "days must be 7, 30, 90, 365, or all")),
    }
}

/// `filters.days` value: the number, or "all".
pub(crate) fn days_value(days: Option<i32>) -> Value {
    days.map_or_else(|| json!("all"), |d| json!(d))
}

/// Case-folded sort key.
pub(crate) fn fold(value: &str) -> String {
    value.to_lowercase()
}

/// Lowercase ASCII alphanumerics separated by single spaces, for matching AI names.
pub fn normalized_ai_name(value: &str) -> String {
    let lowered = value.to_lowercase();
    let mut out = String::with_capacity(lowered.len());
    let mut pending_space = false;
    for c in lowered.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(c);
        } else {
            pending_space = true;
        }
    }
    out
}

pub(crate) fn iso(timestamp: Option<i64>) -> String {
    timestamp.map(format_iso).unwrap_or_default()
}

pub(crate) fn in_range(day: Option<i32>, start: Option<i32>, end: Option<i32>) -> bool {
    if start.is_none() && end.is_none() {
        return true;
    }
    let Some(day) = day else { return false };
    start.is_none_or(|s| day >= s) && end.is_none_or(|e| day <= e)
}

pub(crate) fn day_of(timestamp: Option<i64>) -> Option<i32> {
    timestamp.map(crate::time::day_of)
}

/// Per-metric sums indexed by [`MetricKind`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Totals([i64; 8]);

impl Totals {
    pub fn add(&mut self, kind: MetricKind, value: i64) {
        self.0[kind as usize] += value;
    }

    pub fn get(&self, kind: MetricKind) -> i64 {
        self.0[kind as usize]
    }
}

/// A map that iterates in insertion order.
pub(crate) struct Ordered<K, V> {
    index: HashMap<K, usize>,
    pub entries: Vec<(K, V)>,
}

impl<K: Hash + Eq + Clone, V: Default> Ordered<K, V> {
    pub fn new() -> Self {
        Self { index: HashMap::new(), entries: Vec::new() }
    }

    pub fn entry(&mut self, key: K) -> &mut V {
        let len = self.entries.len();
        let position = *self.index.entry(key.clone()).or_insert(len);
        if position == len {
            self.entries.push((key, V::default()));
        }
        &mut self.entries[position].1
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.index.get(key).map(|&i| &self.entries[i].1)
    }

    pub fn contains(&self, key: &K) -> bool {
        self.index.contains_key(key)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_and_decoding() {
        let params = Params::parse("days=30&repository_id=github.com%3AR_kg%3D&q=a+b&empty=");
        assert_eq!(params.get("repository_id"), Some("github.com:R_kg="));
        assert_eq!(params.get("q"), Some("a b"));
        assert_eq!(params.get("empty"), None);
        assert_eq!(params.get_or("days", "7"), "30");
        assert_eq!(percent_decode("github%3A123"), "github:123");
        assert_eq!(normalized_ai_name("  Claude_Code!! "), "claude code");
        assert_eq!(parse_days("all"), Ok(None));
        assert_eq!(parse_days("12").unwrap_err().status, 422);
    }
}
