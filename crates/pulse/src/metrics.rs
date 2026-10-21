//! Daily contribution metrics and company credit.
//!
//! Only commits listed on a tracked pull request count. Git also returns direct
//! pushes and imported history; those stay in the state but are not counted.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use pulse_core::snapshot::{CompanyMetricKind, MetricKind};
use pulse_core::state::{Commit, Contributor, PullRequest, PullRequestReview, State};
use pulse_core::time::{Ts, format_date};

use crate::attribution::{
    ResolvedTrailer, TrailerRole, ai_contributor_names, ai_model_identity, contributor_alias_index, normalized_ai_name,
    resolve_trailers,
};
use crate::companies::CompanyCatalog;

/// Shared lookups over one state snapshot.
pub struct Facts<'a> {
    pub state: &'a State,
    pub contributors: HashMap<&'a str, &'a Contributor>,
    /// Uppercased identity kind per contributor (`BOT` for flagged bots without one).
    kinds: HashMap<&'a str, String>,
    pub aliases: HashMap<String, HashSet<String>>,
    pub known_ai_names: HashSet<String>,
    pub pull_requests: HashMap<(&'a str, &'a str), &'a PullRequest>,
    pub commits: HashMap<(&'a str, &'a str), &'a Commit>,
    /// Commits per pull request, for links whose PR and commit both exist.
    pub commits_by_pull_request: HashMap<(&'a str, &'a str), Vec<&'a Commit>>,
    /// Target branches of the PRs each linked commit belongs to.
    pub branches_by_commit: HashMap<(&'a str, &'a str), BTreeSet<&'a str>>,
    /// Commits linked to at least one tracked pull request.
    pub linked_commits: HashSet<(&'a str, &'a str)>,
    /// Resolved trailers of every linked commit.
    trailers: HashMap<(&'a str, &'a str), Vec<ResolvedTrailer>>,
    pub reviews_by_pull_request: HashMap<(&'a str, &'a str), Vec<&'a PullRequestReview>>,
}

impl<'a> Facts<'a> {
    pub fn new(state: &'a State) -> Self {
        let contributors: HashMap<&str, &Contributor> =
            state.contributors.iter().filter(|c| !c.id.is_empty()).map(|c| (c.id.as_str(), c)).collect();
        let kinds = contributors
            .iter()
            .map(|(&id, c)| {
                let kind = if !c.identity_kind.is_empty() {
                    c.identity_kind.to_uppercase()
                } else if c.is_bot {
                    "BOT".to_string()
                } else {
                    "UNKNOWN".to_string()
                };
                (id, kind)
            })
            .collect();
        let pull_requests = state
            .pull_requests
            .iter()
            .map(|pr| ((pr.repository_id.as_str(), pr.provider_id.as_str()), pr))
            .collect::<HashMap<_, _>>();
        let commits =
            state.commits.iter().map(|c| ((c.repository_id.as_str(), c.sha.as_str()), c)).collect::<HashMap<_, _>>();
        let mut commits_by_pull_request: HashMap<_, Vec<&Commit>> = HashMap::new();
        let mut branches_by_commit: HashMap<_, BTreeSet<&str>> = HashMap::new();
        let mut linked_commits = HashSet::new();
        for link in &state.pull_request_commits {
            let pr_key = (link.repository_id.as_str(), link.pull_request_id.as_str());
            let commit_key = (link.repository_id.as_str(), link.commit_sha.as_str());
            let Some(pr) = pull_requests.get(&pr_key) else { continue };
            if let Some(commit) = commits.get(&commit_key) {
                commits_by_pull_request.entry(pr_key).or_default().push(*commit);
                linked_commits.insert(commit_key);
            }
            if !pr.target_branch.is_empty() {
                branches_by_commit.entry(commit_key).or_default().insert(pr.target_branch.as_str());
            }
        }
        let mut reviews_by_pull_request: HashMap<_, Vec<&PullRequestReview>> = HashMap::new();
        for review in &state.pull_request_reviews {
            reviews_by_pull_request
                .entry((review.repository_id.as_str(), review.pull_request_id.as_str()))
                .or_default()
                .push(review);
        }
        let aliases = contributor_alias_index(&state.contributors);
        let known_ai_names = ai_contributor_names(&state.contributors);
        let trailers = state
            .commits
            .iter()
            .filter(|c| linked_commits.contains(&(c.repository_id.as_str(), c.sha.as_str())))
            .map(|c| {
                ((c.repository_id.as_str(), c.sha.as_str()), resolve_trailers(&c.message, &aliases, &known_ai_names))
            })
            .collect();
        Self {
            state,
            contributors,
            kinds,
            aliases,
            known_ai_names,
            pull_requests,
            commits,
            commits_by_pull_request,
            branches_by_commit,
            linked_commits,
            trailers,
            reviews_by_pull_request,
        }
    }

    /// `HUMAN`, `BOT`, `AI`, or `UNKNOWN`; empty for an unknown contributor ID.
    pub fn kind(&self, contributor_id: &str) -> &str {
        self.kinds.get(contributor_id).map_or("", String::as_str)
    }

    /// The resolved `Co-Authored-By` / `Assisted-By` trailers of a linked commit.
    pub fn commit_trailers(&self, commit: &'a Commit) -> &[ResolvedTrailer] {
        self.trailers.get(&(commit.repository_id.as_str(), commit.sha.as_str())).map_or(&[], Vec::as_slice)
    }

    /// Display name, then login, used for AI identity names.
    pub fn label(&self, contributor_id: &str) -> Option<&'a str> {
        let c = self.contributors.get(contributor_id)?;
        [c.display_name.as_str(), c.login.as_str()].into_iter().find(|v| !v.is_empty())
    }

    /// When each counted reviewer first reviewed a pull request. Reviews by the
    /// author and by bots and AI accounts do not count.
    pub fn reviewers(&self, pr: &PullRequest) -> BTreeMap<&'a str, Ts> {
        let mut first: BTreeMap<&'a str, Ts> = BTreeMap::new();
        let key = (pr.repository_id.as_str(), pr.provider_id.as_str());
        for review in self.reviews_by_pull_request.get(&key).into_iter().flatten() {
            let reviewer = review.reviewer_contributor_id.as_str();
            let Some(at) = review.submitted_at else { continue };
            if reviewer.is_empty()
                || reviewer == pr.author_contributor_id
                || matches!(self.kind(reviewer), "BOT" | "AI")
            {
                continue;
            }
            first.entry(reviewer).and_modify(|t| *t = (*t).min(at)).or_insert(at);
        }
        first
    }

    pub fn counted_commits(&self) -> impl Iterator<Item = &'a Commit> + '_ {
        self.state.commits.iter().filter(|c| self.linked_commits.contains(&(c.repository_id.as_str(), c.sha.as_str())))
    }
}

pub type MetricKey = (i32, String, String, MetricKind, String);

/// Aggregated `(date, repository, contributor key, metric, branch) -> value`.
pub fn build_daily_metrics(facts: &Facts) -> BTreeMap<MetricKey, i64> {
    let repositories: HashSet<&str> = facts.state.repositories.iter().map(|r| r.repository_id.as_str()).collect();
    let mut aggregates: BTreeMap<MetricKey, i64> = BTreeMap::new();
    let mut add = |at: Option<Ts>, repo: &str, contributor: &str, kind: MetricKind, value: i64, branch: &str| {
        let Some(at) = at else { return };
        // Bot activity stays in the state but does not count as contribution.
        if facts.kind(contributor) == "BOT" || !repositories.contains(repo) {
            return;
        }
        *aggregates
            .entry((at.day(), repo.to_string(), contributor.to_string(), kind, branch.to_string()))
            .or_default() += value;
    };

    for pr in &facts.state.pull_requests {
        let repo = pr.repository_id.as_str();
        let author = pr.author_contributor_id.as_str();
        let branch = pr.target_branch.as_str();
        let mut both = |at: Option<Ts>, contributor: &str, kind: MetricKind, value: i64| {
            add(at, repo, contributor, kind, value, "");
            if !branch.is_empty() {
                add(at, repo, contributor, kind, value, branch);
            }
        };
        both(pr.created_at, author, MetricKind::PullRequestsCreated, 1);
        if pr.merged_at.is_some() {
            both(pr.merged_at, author, MetricKind::PullRequestsMerged, 1);
            both(pr.merged_at, author, MetricKind::Additions, pr.additions);
            for participant in shared_participants(facts, pr) {
                both(pr.merged_at, &participant, MetricKind::SharedAdditions, pr.additions);
                both(pr.merged_at, &participant, MetricKind::SharedDeletions, pr.deletions);
            }
            both(pr.merged_at, author, MetricKind::Deletions, pr.deletions);
        }
        for (reviewer, at) in facts.reviewers(pr) {
            both(Some(at), reviewer, MetricKind::Reviews, 1);
        }
    }
    for commit in facts.counted_commits() {
        let key = (commit.repository_id.as_str(), commit.sha.as_str());
        let author = commit.author_contributor_id.as_str();
        add(commit.committed_at, key.0, author, MetricKind::Commits, 1, "");
        for branch in facts.branches_by_commit.get(&key).into_iter().flatten() {
            add(commit.committed_at, key.0, author, MetricKind::Commits, 1, branch);
        }
    }
    aggregates
}

/// People and AI identities credited with a merged PR's full line diff.
fn shared_participants(facts: &Facts, pr: &PullRequest) -> BTreeSet<String> {
    let mut participants = BTreeSet::new();
    let add_ai = |participants: &mut BTreeSet<String>, name: &str| {
        let identity = ai_model_identity(name);
        if !identity.key.is_empty() {
            participants.insert(format!("ai-model:{}", identity.key));
            participants.insert(format!("ai-provider:{}", normalized_ai_name(&identity.provider)));
        }
    };
    let add_contributor = |participants: &mut BTreeSet<String>, id: &str| match facts.kind(id) {
        _ if id.is_empty() => {}
        "BOT" => {}
        "AI" => add_ai(participants, facts.label(id).unwrap_or_default()),
        _ => {
            participants.insert(id.to_string());
        }
    };

    add_contributor(&mut participants, &pr.author_contributor_id);
    let key = (pr.repository_id.as_str(), pr.provider_id.as_str());
    for commit in facts.commits_by_pull_request.get(&key).into_iter().flatten() {
        add_contributor(&mut participants, &commit.author_contributor_id);
        for trailer in facts.commit_trailers(commit) {
            if trailer.is_ai || facts.kind(&trailer.resolved) == "AI" {
                add_ai(&mut participants, facts.label(&trailer.resolved).unwrap_or(&trailer.person));
            } else if trailer.role == TrailerRole::CoAuthoredBy && !trailer.resolved.is_empty() {
                add_contributor(&mut participants, &trailer.resolved);
            }
        }
    }
    participants
}

pub type CompanyKey = (i32, String, String, String, CompanyMetricKind);
pub type CoverageKey = (i32, String, String, CompanyMetricKind);

#[derive(Default)]
pub struct CompanyMetrics {
    /// `(date, repository, branch, company or "independent", metric) -> value`
    pub values: BTreeMap<CompanyKey, i64>,
    /// `(date, repository, branch, metric) -> (all events, reviewed events)`
    pub coverage: BTreeMap<CoverageKey, (i64, i64)>,
}

/// Company credit with deduplication inside each company before aggregation.
pub fn build_company_metrics(facts: &Facts, catalog: &CompanyCatalog) -> CompanyMetrics {
    // Unknown identities ("") count as human: unresolved co-authors keep coverage honest.
    let human = |id: &str| !matches!(facts.kind(id), "AI" | "BOT");
    let mut participants: HashMap<(&str, &str), BTreeSet<String>> = HashMap::new();
    for commit in facts.counted_commits() {
        let author = commit.author_contributor_id.as_str();
        let mut ids = BTreeSet::new();
        if human(author) {
            ids.insert(author.to_string());
        }
        for trailer in facts.commit_trailers(commit) {
            if trailer.role == TrailerRole::CoAuthoredBy && !trailer.is_ai && human(&trailer.resolved) {
                ids.insert(trailer.resolved.clone());
            }
        }
        participants.insert((commit.repository_id.as_str(), commit.sha.as_str()), ids);
    }

    let mut result = CompanyMetrics::default();
    let mut emit = |repo: &str,
                    at: Option<Ts>,
                    ids: &BTreeSet<String>,
                    branches: &BTreeSet<&str>,
                    metrics: &[(CompanyMetricKind, i64)]| {
        let Some(at) = at else { return };
        if ids.is_empty() {
            return;
        }
        let day = at.day();
        let date = format_date(day);
        let companies: BTreeSet<String> = ids
            .iter()
            .map(|id| {
                let company = catalog.historical(id, &date);
                if company.is_empty() { "independent".to_string() } else { company }
            })
            .collect();
        let reviewed = ids.iter().all(|id| catalog.effective(id, &date).is_some());
        let scopes = std::iter::once("").chain(branches.iter().copied().filter(|b| !b.is_empty()));
        for branch in scopes.collect::<BTreeSet<_>>() {
            for &(kind, value) in metrics {
                let coverage = result.coverage.entry((day, repo.to_string(), branch.to_string(), kind)).or_default();
                coverage.0 += value;
                if reviewed {
                    coverage.1 += value;
                }
                for company in &companies {
                    *result
                        .values
                        .entry((day, repo.to_string(), branch.to_string(), company.clone(), kind))
                        .or_default() += value;
                }
            }
        }
    };

    for commit in facts.counted_commits() {
        let key = (commit.repository_id.as_str(), commit.sha.as_str());
        let branches = facts.branches_by_commit.get(&key).cloned().unwrap_or_default();
        emit(key.0, commit.committed_at, &participants[&key], &branches, &[(CompanyMetricKind::Commits, 1)]);
    }
    let mut pr_commits: HashMap<(&str, &str), Vec<&str>> = HashMap::new();
    for link in &facts.state.pull_request_commits {
        pr_commits
            .entry((link.repository_id.as_str(), link.pull_request_id.as_str()))
            .or_default()
            .push(link.commit_sha.as_str());
    }
    for pr in &facts.state.pull_requests {
        let repo = pr.repository_id.as_str();
        let author = pr.author_contributor_id.as_str();
        let ids: BTreeSet<String> = if human(author) { BTreeSet::from([author.to_string()]) } else { BTreeSet::new() };
        let branches: BTreeSet<&str> =
            if pr.target_branch.is_empty() { BTreeSet::new() } else { BTreeSet::from([pr.target_branch.as_str()]) };
        emit(repo, pr.created_at, &ids, &branches, &[(CompanyMetricKind::PullRequestsOpened, 1)]);
        let mut shared = ids.clone();
        for sha in pr_commits.get(&(repo, pr.provider_id.as_str())).into_iter().flatten() {
            if let Some(people) = participants.get(&(repo, *sha)) {
                shared.extend(people.iter().cloned());
            }
        }
        emit(
            repo,
            pr.merged_at,
            &shared,
            &branches,
            &[
                (CompanyMetricKind::PullRequestsMerged, 1),
                (CompanyMetricKind::SharedAdditions, pr.additions),
                (CompanyMetricKind::SharedDeletions, pr.deletions),
            ],
        );
        for (reviewer, at) in facts.reviewers(pr) {
            let ids = BTreeSet::from([reviewer.to_string()]);
            emit(repo, Some(at), &ids, &branches, &[(CompanyMetricKind::Reviews, 1)]);
        }
    }
    result
}
