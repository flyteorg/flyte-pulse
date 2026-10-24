//! Overview, repository list, and repository detail (`_dashboard_data`,
//! `repository_list`, `repository_detail`).

use std::cmp::Reverse;
use std::collections::HashMap;

use serde_json::{Value, json};

use super::languages::{by_size, language_rows};
use super::{Dataset, Ordered, Totals, View, day_of, days_value, fold, in_range, iso};
use crate::snapshot::{MetricKind, NONE, Pr};
use crate::time::format_date;

/// Which metric rows make up the "snapshot" a query sees. Repository pages use
/// their repository's (and branch's) rows only, which also bound "all" dates.
#[derive(Clone, Copy)]
pub(crate) enum Scope {
    All,
    Repo(u32),
    RepoBranch(u32, u32),
}

const ACTIVITY_METRICS: [MetricKind; 5] = [
    MetricKind::PullRequestsCreated,
    MetricKind::PullRequestsMerged,
    MetricKind::Commits,
    MetricKind::SharedAdditions,
    MetricKind::SharedDeletions,
];

/// First and last recorded activity per person, overall and per repository.
#[derive(Default)]
pub(crate) struct DateBounds {
    pub first: HashMap<u32, i32>,
    pub last: HashMap<u32, i32>,
    pub first_by_repo: HashMap<u32, HashMap<u32, i32>>,
    pub last_by_repo: HashMap<u32, HashMap<u32, i32>>,
}

impl Dataset {
    /// First and last activity from repository-wide rows only, so tracked branches
    /// do not double count; shared line credit includes co-authors.
    pub(crate) fn date_bounds(&self, rows: impl Iterator<Item = u32>) -> DateBounds {
        let mut bounds = DateBounds::default();
        for index in rows {
            let row = &self.snapshot.metrics[index as usize];
            if row.branch != 0 || !ACTIVITY_METRICS.contains(&row.kind) || row.value <= 0 || row.key == NONE {
                continue;
            }
            let first = bounds.first.entry(row.key).or_insert(row.date);
            *first = (*first).min(row.date);
            let last = bounds.last.entry(row.key).or_insert(row.date);
            *last = (*last).max(row.date);
            let first = bounds.first_by_repo.entry(row.key).or_default().entry(row.repo).or_insert(row.date);
            *first = (*first).min(row.date);
            let last = bounds.last_by_repo.entry(row.key).or_default().entry(row.repo).or_insert(row.date);
            *last = (*last).max(row.date);
        }
        bounds
    }

    /// Display name for a repository row, with a branch label when it is one of several.
    pub(crate) fn repository_label(&self, repo: u32, branch: Option<(&str, &str)>) -> String {
        let name = self.repo(repo).full_name();
        match branch {
            Some((branch, label)) => format!("{name} · {}", if label.is_empty() { branch } else { label }),
            None => name,
        }
    }

    /// Merged pull requests in range that `include` selects (bot-authored ones are not
    /// counted), how many had a review by someone other than the author before they
    /// merged, and the median hours from opening to that first review.
    pub(crate) fn review_coverage(
        &self,
        include: impl Fn(&Pr) -> bool,
        start: i32,
        end: i32,
    ) -> (i64, i64, Option<f64>) {
        let (mut merged, mut hours) = (0, Vec::new());
        for pr in &self.snapshot.pull_requests {
            let Some(merged_at) = pr.merged_at else { continue };
            if !in_range(day_of(Some(merged_at)), Some(start), Some(end)) || self.is_bot(pr.author) || !include(pr) {
                continue;
            }
            merged += 1;
            if let Some(reviewed_at) = pr.first_review_at.filter(|&at| at <= merged_at) {
                let opened = pr.created_at.unwrap_or(reviewed_at);
                hours.push((reviewed_at - opened).max(0) as f64 / 3600.0);
            }
        }
        hours.sort_by(f64::total_cmp);
        let median = match hours.len() {
            0 => None,
            n if n % 2 == 1 => Some(hours[n / 2]),
            n => Some((hours[n / 2 - 1] + hours[n / 2]) / 2.0),
        };
        (merged, hours.len() as i64, median.map(|h| (h * 10.0).round() / 10.0))
    }

    pub(crate) fn pull_request_view(&self, index: u32) -> Value {
        let pr = &self.snapshot.pull_requests[index as usize];
        let author_id = self.key_path(pr.author);
        json!({
            "repository_id": self.repo_path(pr.repo),
            "provider_id": pr.provider_id,
            "repository": self.repo(pr.repo).full_name(),
            "number": pr.number,
            "title": if pr.title.is_empty() { "Untitled pull request" } else { pr.title.as_str() },
            "state": if pr.state.is_empty() { "unknown" } else { pr.state.as_str() },
            "created_at": iso(pr.created_at),
            "merged_at": iso(pr.merged_at),
            "target_branch": self.branch(pr.branch),
            "author_id": author_id,
            "author_name": self.name_or(pr.author, author_id),
            "author_login": self.login(pr.author),
            "author_kind": self.person(pr.author).map_or("UNKNOWN", |p| p.kind.as_str()),
            "additions": pr.additions,
            "deletions": pr.deletions,
            "commit_count": pr.commit_count,
            "url": self.pr_url(pr),
        })
    }

    /// The name a ranking shows: display name, then `@login`, then the identifier.
    fn ranking_name(&self, key: u32) -> String {
        match self.person(key) {
            Some(p) if !p.display_name.is_empty() => p.display_name.clone(),
            Some(p) if !p.login.is_empty() => format!("@{}", p.login),
            _ => self.key_path(key).to_string(),
        }
    }
}

fn contributor_row(view: &View, key: u32, values: &Totals) -> Value {
    let data = view.data;
    json!({
        "id": data.key_path(key),
        "name": data.ranking_name(key),
        "login": data.login(key),
        "identity_kind": data.person(key).map_or("UNKNOWN", |p| p.kind.as_str()),
        "pull_requests_opened": values.get(MetricKind::PullRequestsCreated),
        "pull_requests_merged": values.get(MetricKind::PullRequestsMerged),
        "commits": values.get(MetricKind::Commits),
        "shared_additions": values.get(MetricKind::SharedAdditions),
        "shared_deletions": values.get(MetricKind::SharedDeletions),
    })
}

fn int(value: &Value, key: &str) -> i64 {
    value[key].as_i64().unwrap_or(0)
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_string()
}

/// Descending by score, then descending by case-folded name; ties keep their order.
pub(crate) fn sort_by_score(rows: &mut [Value], score: impl Fn(&Value) -> i64) {
    rows.sort_by_cached_key(|row| Reverse((score(row), fold(&text(row, "name")))));
}

impl View<'_> {
    pub(crate) fn dashboard(
        &self,
        scope: Scope,
        days: Option<i32>,
        repository: Option<&str>,
        target_branch: Option<&str>,
    ) -> Value {
        let data = self.data;
        let today = self.today;
        let repo_filter = repository.map(|id| data.repo_index(id));
        let target = target_branch.map(|name| data.branch_filter(name));

        // The rows this query's "snapshot" contains.
        let metrics = &data.snapshot.metrics;
        let partition = |repo: u32, branch: Option<u32>| -> Vec<u32> {
            data.metrics_by_repo[repo as usize]
                .iter()
                .copied()
                .filter(|&i| branch.is_none_or(|b| metrics[i as usize].branch == b))
                .collect()
        };
        let snapshot_rows: Vec<u32> = match scope {
            Scope::All => Vec::new(),
            Scope::Repo(repo) => partition(repo, None),
            Scope::RepoBranch(repo, branch) => partition(repo, Some(branch)),
        };
        let metric_dates = match scope {
            Scope::All => data.metric_dates,
            _ => snapshot_rows.iter().fold(None, |acc: Option<(i32, i32)>, &i| {
                let date = metrics[i as usize].date;
                Some(acc.map_or((date, date), |(f, l)| (f.min(date), l.max(date))))
            }),
        };
        let first_date = metric_dates.map_or(today, |d| d.0);
        let last_date = metric_dates.map_or(today, |d| d.1);
        let start = match days {
            Some(days) => today - (days - 1),
            None => first_date,
        };
        let end = today.max(last_date);

        let scoped: Vec<u32> = match (scope, repo_filter) {
            (_, Some(None)) => Vec::new(),
            (Scope::All, Some(Some(repo))) => data.metrics_by_repo[repo as usize].clone(),
            (Scope::All, None) => (0..metrics.len() as u32).collect(),
            (_, Some(Some(repo))) => snapshot_rows.into_iter().filter(|&i| metrics[i as usize].repo == repo).collect(),
            (_, None) => snapshot_rows,
        };

        let mut totals = Totals::default();
        let mut daily: HashMap<i32, Totals> = HashMap::new();
        let mut contributor_totals: Ordered<u32, Totals> = Ordered::new();
        let mut community_totals: Ordered<u32, Totals> = Ordered::new();
        let mut reviewer_totals: Ordered<u32, i64> = Ordered::new();
        let mut repository_totals: Ordered<u32, Totals> = Ordered::new();
        let mut branch_totals: HashMap<(u32, u32), Totals> = HashMap::new();
        for &index in &scoped {
            let row = &data.snapshot.metrics[index as usize];
            if target.is_some_and(|t| row.branch != t) {
                continue;
            }
            if row.date < start || row.date > end {
                continue;
            }
            if row.branch != 0 && target.is_none() {
                branch_totals.entry((row.repo, row.branch)).or_default().add(row.kind, row.value);
                continue;
            }
            if target.is_some() && row.branch == 0 {
                continue;
            }
            totals.add(row.kind, row.value);
            // Reviewing is credited separately; it does not make anyone an active contributor.
            if row.kind == MetricKind::Reviews {
                if data.is_human(row.key) {
                    *reviewer_totals.entry(row.key) += row.value;
                }
                continue;
            }
            daily.entry(row.date).or_default().add(row.kind, row.value);
            // AI attribution keys (`ai-model:`, `ai-provider:`) carry shared diff metrics
            // for the AI views; they are not contributors.
            if data.person(row.key).is_some() {
                contributor_totals.entry(row.key).add(row.kind, row.value);
                if data.is_community(row.key, row.date) {
                    community_totals.entry(row.key).add(row.kind, row.value);
                }
            }
            repository_totals.entry(row.repo).add(row.kind, row.value);
        }

        let daily: Vec<Value> = (start..=end)
            .map(|day| {
                let values = daily.get(&day).copied().unwrap_or_default();
                json!({
                    "date": format_date(day),
                    "pull_requests_merged": values.get(MetricKind::PullRequestsMerged),
                    "commits": values.get(MetricKind::Commits),
                })
            })
            .collect();

        let ranked = |totals: &Ordered<u32, Totals>| -> Vec<(u32, Value)> {
            let mut entries: Vec<&(u32, Totals)> = totals.entries.iter().collect();
            // Merged pull requests first, then commits and opened pull requests, so lists
            // cut to the top few keep the contributors with the most merged work.
            entries.sort_by_cached_key(|(key, values)| {
                let score = (
                    values.get(MetricKind::PullRequestsMerged),
                    values.get(MetricKind::Commits),
                    values.get(MetricKind::PullRequestsCreated),
                );
                (Reverse(score), fold(&data.ranking_name(*key)))
            });
            entries.into_iter().map(|(key, values)| (*key, contributor_row(self, *key, values))).collect()
        };
        let top = ranked(&contributor_totals);
        // Community activity is what humans did while not at a maintainer company.
        let community_top: Vec<Value> = ranked(&community_totals).into_iter().map(|(_, v)| v).collect();
        let maintainers: Vec<&str> =
            data.snapshot.companies.iter().filter(|c| c.maintainer).map(|c| c.canonical_name.as_str()).collect();
        let human_top: Vec<(u32, Value)> = top.iter().filter(|(key, _)| data.is_human(*key)).cloned().collect();
        let active_human = contributor_totals.entries.iter().filter(|(key, _)| data.is_human(*key)).count();

        let bounds = data.date_bounds(scoped.iter().copied());
        let selected_first: HashMap<u32, i32> = match repo_filter {
            Some(Some(repo)) => {
                bounds.first_by_repo.iter().filter_map(|(key, dates)| dates.get(&repo).map(|d| (*key, *d))).collect()
            }
            Some(None) => HashMap::new(),
            None => bounds.first,
        };
        let is_new = |date: i32| start <= date && date <= end;
        let new_humans = selected_first.iter().filter(|(key, date)| is_new(**date) && data.is_human(**key)).count();
        let mut new_top: Vec<(i32, Value)> = human_top
            .iter()
            .filter_map(|(key, row)| {
                let date = *selected_first.get(key)?;
                if !is_new(date) {
                    return None;
                }
                let mut row = row.clone();
                row["first_contribution_date"] = json!(format_date(date));
                Some((date, row))
            })
            .collect();
        new_top.sort_by(|(da, a), (db, b)| {
            let score = |v: &Value| int(v, "pull_requests_merged") + int(v, "commits");
            (db, score(b)).cmp(&(da, score(a)))
        });

        let repository_row = |repo: u32, values: &Totals, branch: Option<(&str, &str)>| {
            json!({
                "id": data.repo_path(repo),
                "name": data.repository_label(repo, branch),
                "target_branch": branch.map_or("", |b| b.0),
                "branch_label": branch.map_or("", |b| b.1),
                "pull_requests_opened": values.get(MetricKind::PullRequestsCreated),
                "pull_requests_merged": values.get(MetricKind::PullRequestsMerged),
                "commits": values.get(MetricKind::Commits),
                "additions": values.get(MetricKind::Additions),
                "deletions": values.get(MetricKind::Deletions),
            })
        };
        let mut repository_activity = Vec::new();
        for (repo, values) in &repository_totals.entries {
            if let Some(branch) = target_branch {
                repository_activity.push(repository_row(*repo, values, Some((branch, ""))));
                continue;
            }
            let branches = &data.repo(*repo).target_branches;
            if branches.len() <= 1 {
                repository_activity.push(repository_row(*repo, values, None));
                continue;
            }
            for branch in branches {
                let index = data.branch_filter(&branch.name);
                let values = branch_totals.get(&(*repo, index)).copied().unwrap_or_default();
                repository_activity.push(repository_row(*repo, &values, Some((&branch.name, &branch.label))));
            }
        }
        sort_by_score(&mut repository_activity, |v| int(v, "pull_requests_merged") + int(v, "commits"));

        let mut reviewers: Vec<Value> = reviewer_totals
            .entries
            .iter()
            .map(|(key, reviews)| {
                let mut row = contributor_row(self, *key, &contributor_totals.get(key).copied().unwrap_or_default());
                row["reviews"] = json!(reviews);
                row
            })
            .collect();
        sort_by_score(&mut reviewers, |v| int(v, "reviews"));

        let repos_in_view: Vec<u32> = match (scope, repo_filter) {
            (_, Some(Some(repo))) | (Scope::Repo(repo) | Scope::RepoBranch(repo, _), None) => vec![repo],
            (Scope::All, None) => (0..data.snapshot.repositories.len() as u32).collect(),
            (_, Some(None)) => Vec::new(),
        };
        let (merged, reviewed, median_hours) = data.review_coverage(
            |pr| repos_in_view.contains(&pr.repo) && target.is_none_or(|t| pr.branch == t),
            start,
            end,
        );
        // Code size by language for the repositories in view.
        let languages = language_rows(repos_in_view.iter().map(|&r| by_size(&data.repo(r).languages)));

        let meta = &data.snapshot.meta;
        json!({
            "metrics_generated_at": iso(Some(meta.generated_at)),
            "raw_generated_at": iso(Some(meta.raw_generated_at)),
            "metrics_version": meta.version,
            "raw_version": meta.raw_version,
            "serving_version": meta.version,
            "repositories": data.repository_options(),
            "summary": {
                "pull_requests_opened": totals.get(MetricKind::PullRequestsCreated),
                "pull_requests_merged": totals.get(MetricKind::PullRequestsMerged),
                "commits": totals.get(MetricKind::Commits),
                "active_contributors": contributor_totals.len(),
                "active_human_contributors": active_human,
                "new_human_contributors": new_humans,
                "additions": totals.get(MetricKind::Additions),
                "deletions": totals.get(MetricKind::Deletions),
                "active_repositories": repository_totals.len(),
                "reviews": totals.get(MetricKind::Reviews),
                "active_reviewers": reviewer_totals.len(),
                "merged_pull_requests_reviewed": reviewed,
                "merged_pull_requests_for_coverage": merged,
                "review_coverage": (merged > 0).then(|| (reviewed as f64 / merged as f64 * 10_000.0).round() / 10_000.0),
                "median_hours_to_first_review": median_hours,
            },
            "daily": daily,
            "top_contributors": top.iter().take(12).map(|(_, v)| v.clone()).collect::<Vec<_>>(),
            "human_top_contributors": human_top.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
            "new_top_contributors": new_top.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
            "community_top_contributors": community_top,
            "maintainer_companies": maintainers,
            "top_reviewers": reviewers,
            "ai_top_contributors": [],
            "repository_activity": repository_activity,
            "languages": languages,
        })
    }

    pub(crate) fn repository_list(&self) -> Vec<Value> {
        let data = self.data;
        let mut output = Vec::new();
        for (index, repo) in data.snapshot.repositories.iter().enumerate() {
            for branch in repo.branch_views() {
                let activity = self.dashboard(Scope::All, Some(365), Some(&repo.id), branch.map(|b| b.name.as_str()));
                output.push(json!({
                    "id": data.repo_path(index as u32),
                    "name": data.repository_label(index as u32, branch.map(|b| (b.name.as_str(), b.label.as_str()))),
                    "target_branch": branch.map_or("", |b| b.name.as_str()),
                    "branch_label": branch.map_or("", |b| b.label.as_str()),
                    "status": repo.status_label(),
                    "summary": activity["summary"],
                }));
            }
        }
        sort_by_score(&mut output, |v| int(&v["summary"], "pull_requests_merged") + int(&v["summary"], "commits"));
        output
    }

    pub(crate) fn repository_detail(&self, id: &str, days: Option<i32>, target_branch: Option<&str>) -> Option<Value> {
        let data = self.data;
        let repo_index = data.repo_index(id)?;
        let repo = data.repo(repo_index);
        let branch_index = target_branch.map(|b| data.branch_filter(b));
        let scope = match branch_index {
            Some(branch) => Scope::RepoBranch(repo_index, branch),
            None => Scope::Repo(repo_index),
        };
        let activity = self.dashboard(scope, days, Some(id), target_branch);
        let start = days.map(|d| self.today - (d - 1));
        let recent: Vec<Value> = data
            .prs_newest_first
            .iter()
            .copied()
            .filter(|&i| {
                let pr = &data.snapshot.pull_requests[i as usize];
                pr.repo == repo_index
                    && branch_index.is_none_or(|b| pr.branch == b)
                    && (in_range(day_of(pr.created_at), start, Some(self.today))
                        || in_range(day_of(pr.merged_at), start, Some(self.today)))
            })
            .take(50)
            .map(|i| data.pull_request_view(i))
            .collect();
        let selected_label = repo
            .target_branches
            .iter()
            .find(|b| Some(b.name.as_str()) == target_branch)
            .map_or("", |b| b.label.as_str());
        Some(json!({
            "repository": {
                "id": data.repo_path(repo_index),
                "name": repo.full_name(),
                "owner": repo.owner,
                "repository_name": repo.name,
                "status": repo.status_label(),
                "module": repo.module,
                "target_branches": repo.target_branches.iter().map(|b| json!({"name": b.name, "label": b.label})).collect::<Vec<_>>(),
                "selected_target_branch": target_branch.unwrap_or(""),
                "selected_branch_label": selected_label,
            },
            "filters": {"days": days_value(days), "target_branch": target_branch.unwrap_or("")},
            "summary": activity["summary"],
            "daily": activity["daily"],
            "top_contributors": activity["top_contributors"],
            "top_reviewers": activity["top_reviewers"].as_array().map(|rows| rows.iter().take(12).cloned().collect::<Vec<_>>()),
            "languages": activity["languages"],
            "recent_pull_requests": recent,
        }))
    }
}
