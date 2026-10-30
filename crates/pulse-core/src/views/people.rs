//! Contributor directory and contributor detail (`contributor_list`,
//! `contributor_detail`, `_contributor_ai_usage`, `_contributor_ai_provider_lines`).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde_json::{Value, json};

use super::activity::sort_by_score;
use super::ai::Identity;
use super::company::profile_affiliations;
use super::languages::language_rows;
use super::{Ordered, Totals, View, day_of, days_value, fold, in_range};
use crate::snapshot::{AiRole, MetricKind, NONE};
use crate::time::format_date;

impl View<'_> {
    pub(crate) fn contributor_index(&self) -> Value {
        json!({"users": self.contributor_list(), "repositories": self.data.repository_options()})
    }

    fn contributor_list(&self) -> Vec<Value> {
        let data = self.data;
        let mut totals: HashMap<u32, Totals> = HashMap::new();
        for row in &data.snapshot.metrics {
            if row.branch == 0 && row.key != NONE {
                totals.entry(row.key).or_default().add(row.kind, row.value);
            }
        }
        let bounds = data.date_bounds(0..data.snapshot.metrics.len() as u32);
        let mut authored: HashMap<u32, usize> = HashMap::new();
        let mut committed: HashMap<u32, usize> = HashMap::new();
        for commit in &data.snapshot.commits {
            if commit.author != NONE {
                *authored.entry(commit.author).or_default() += 1;
            }
            if commit.committer != NONE {
                *committed.entry(commit.committer).or_default() += 1;
            }
        }
        let repo_dates = |dates: Option<&HashMap<u32, i32>>| -> Value {
            let map: BTreeMap<String, String> = dates
                .into_iter()
                .flatten()
                .map(|(repo, date)| (data.repo_path(*repo).to_string(), format_date(*date)))
                .collect();
            json!(map)
        };
        let mut output = Vec::new();
        for (index, person) in data.snapshot.contributors.iter().enumerate() {
            let key = index as u32;
            if !data.is_human(key) {
                continue;
            }
            let values = totals.get(&key).copied().unwrap_or_default();
            let opened = values.get(MetricKind::PullRequestsCreated);
            let merged = values.get(MetricKind::PullRequestsMerged);
            let commits_authored = authored.get(&key).copied().unwrap_or(0) as i64;
            let commits_committed = committed.get(&key).copied().unwrap_or(0) as i64;
            let shared = values.get(MetricKind::SharedAdditions) + values.get(MetricKind::SharedDeletions);
            if opened == 0 && merged == 0 && commits_authored == 0 && commits_committed == 0 && shared == 0 {
                continue;
            }
            output.push(json!({
                "id": data.key_path(key),
                "name": data.display(key),
                "login": person.login,
                "kind": person.kind,
                "profile_url": person.profile_url,
                "pull_requests_opened": opened,
                "pull_requests_merged": merged,
                "commits_authored": commits_authored,
                "commits_committed": commits_committed,
                "lines_added": values.get(MetricKind::SharedAdditions),
                "lines_removed": values.get(MetricKind::SharedDeletions),
                "activity": opened + merged + commits_authored + commits_committed,
                "first_contribution_date": bounds.first.get(&key).map(|d| format_date(*d)).unwrap_or_default(),
                "first_contribution_by_repository": repo_dates(bounds.first_by_repo.get(&key)),
                "last_contribution_date": bounds.last.get(&key).map(|d| format_date(*d)).unwrap_or_default(),
                "last_contribution_by_repository": repo_dates(bounds.last_by_repo.get(&key)),
            }));
        }
        sort_by_score(&mut output, |v| v["activity"].as_i64().unwrap_or(0));
        output
    }

    pub(crate) fn contributor_detail(
        &self,
        id: &str,
        days: Option<i32>,
        repository: Option<&str>,
        include_history: bool,
    ) -> Option<Value> {
        let data = self.data;
        let key = data.key_index(id).filter(|&k| data.person(k).is_some())?;
        let person = data.person(key)?;
        let today = self.today;
        let selected = repository.map(|r| data.repo_index(r).unwrap_or(NONE));
        let multi_branch = |repo: u32| data.repo(repo).target_branches.len() > 1;

        let rows =
            data.metrics_by_key.get(key as usize).into_iter().flatten().map(|&i| &data.snapshot.metrics[i as usize]);
        let mut repository_options: BTreeSet<u32> = BTreeSet::new();
        let mut global_rows = Vec::new();
        let mut branch_rows = Vec::new();
        for row in rows {
            if row.value != 0
                && matches!(
                    row.kind,
                    MetricKind::Commits
                        | MetricKind::PullRequestsCreated
                        | MetricKind::PullRequestsMerged
                        | MetricKind::Reviews
                )
            {
                repository_options.insert(row.repo);
            }
            if row.branch != 0 {
                if multi_branch(row.repo) {
                    branch_rows.push(row);
                }
                continue;
            }
            global_rows.push(row);
        }
        let selected_ok = |repo: u32| selected.is_none_or(|s| s == repo);
        let start = match days {
            Some(days) => today - (days - 1),
            None => global_rows
                .iter()
                .chain(branch_rows.iter())
                .filter(|row| selected_ok(row.repo))
                .map(|row| row.date)
                .min()
                .unwrap_or(today),
        };

        let mut totals = Totals::default();
        let mut by_date: HashMap<i32, Totals> = HashMap::new();
        let mut repository_metrics: HashMap<(u32, u32), Totals> = HashMap::new();
        // Languages of the repositories this person committed to, weighted by their commits.
        let mut commits_by_repo: HashMap<u32, i64> = HashMap::new();
        for row in &global_rows {
            if !selected_ok(row.repo) || row.date < start || row.date > today {
                continue;
            }
            if !multi_branch(row.repo) {
                repository_metrics.entry((row.repo, 0)).or_default().add(row.kind, row.value);
            }
            totals.add(row.kind, row.value);
            by_date.entry(row.date).or_default().add(row.kind, row.value);
            if row.kind == MetricKind::Commits {
                *commits_by_repo.entry(row.repo).or_default() += row.value;
            }
        }
        for row in &branch_rows {
            if !selected_ok(row.repo) || row.date < start || row.date > today {
                continue;
            }
            repository_metrics.entry((row.repo, row.branch)).or_default().add(row.kind, row.value);
        }
        let languages = language_rows(
            commits_by_repo.iter().map(|(&repo, &commits)| (&data.repo(repo).languages[..], commits as f64)),
        );

        let mut options: Vec<u32> = repository_options.iter().copied().collect();
        options.sort_by_cached_key(|r| fold(&data.repo(*r).full_name()));
        let mut repository_activity = Vec::new();
        for &repo in &options {
            for branch in data.repo(repo).branch_views() {
                let branch_index = branch.map_or(0, |b| data.branch_filter(&b.name));
                let values = repository_metrics.get(&(repo, branch_index)).copied().unwrap_or_default();
                repository_activity.push(json!({
                    "id": data.repo_path(repo),
                    "target_branch": branch.map_or("", |b| b.name.as_str()),
                    "name": data.repository_label(repo, branch.map(|b| (b.name.as_str(), b.label.as_str()))),
                    "commits": values.get(MetricKind::Commits),
                    "pull_requests_opened": values.get(MetricKind::PullRequestsCreated),
                    "pull_requests_merged": values.get(MetricKind::PullRequestsMerged),
                    "reviews": values.get(MetricKind::Reviews),
                }));
            }
        }
        sort_by_score(&mut repository_activity, |v| {
            v["commits"].as_i64().unwrap_or(0) + v["pull_requests_opened"].as_i64().unwrap_or(0)
        });
        let daily: Vec<Value> = (start..=today)
            .map(|day| {
                let values = by_date.get(&day).copied().unwrap_or_default();
                json!({
                    "date": format_date(day),
                    "pull_requests_opened": values.get(MetricKind::PullRequestsCreated),
                    "pull_requests_merged": values.get(MetricKind::PullRequestsMerged),
                    "commits": values.get(MetricKind::Commits),
                })
            })
            .collect();

        let range = |day: Option<i32>| in_range(day, Some(start), Some(today));
        let mut pull_requests = Vec::new();
        let mut commits = Vec::new();
        let mut authored = 0;
        let mut committed = 0;
        let mut ai_usage = json!({"summary": {}, "top_models": []});
        let mut ai_provider_lines = json!([]);
        if include_history {
            pull_requests = data
                .prs_newest_first
                .iter()
                .copied()
                .filter(|&i| {
                    let pr = &data.snapshot.pull_requests[i as usize];
                    pr.author == key && range(day_of(pr.created_at)) && selected_ok(pr.repo)
                })
                .take(100)
                .map(|i| data.pull_request_view(i, self.today))
                .collect();
            let mut matched: Vec<(Option<i64>, u32)> = Vec::new();
            for (index, commit) in data.snapshot.commits.iter().enumerate() {
                if (commit.author != key && commit.committer != key)
                    || !selected_ok(commit.repo)
                    || !range(day_of(commit.committed_at))
                {
                    continue;
                }
                authored += usize::from(commit.author == key);
                committed += usize::from(commit.committer == key);
                matched.push((commit.committed_at, index as u32));
            }
            // Stable, newest first (missing timestamps last); only the newest are shown.
            matched.sort_by(|a, b| b.0.unwrap_or(i64::MIN).cmp(&a.0.unwrap_or(i64::MIN)));
            commits = matched
                .iter()
                .take(100)
                .map(|&(_, index)| {
                    let commit = &data.snapshot.commits[index as usize];
                    let roles: Vec<&str> = [(commit.author == key, "Author"), (commit.committer == key, "Committer")]
                        .into_iter()
                        .filter_map(|(is, role)| is.then_some(role))
                        .collect();
                    let mut row = data.commit_view(index);
                    row["message"] = json!(commit.subject);
                    row["role"] = json!(roles.join(", "));
                    row
                })
                .collect();
            ai_usage = self.contributor_ai_usage(key, start, selected);
            ai_provider_lines = self.contributor_ai_provider_lines(key, start, selected);
        }

        let companies: HashMap<&str, &crate::snapshot::Company> =
            data.snapshot.companies.iter().map(|c| (c.id.as_str(), c)).collect();
        let mut contributor = json!({
            "id": data.key_path(key),
            "name": data.display(key),
            "login": person.login,
            "kind": person.kind,
            "kind_source": person.kind_source,
            "profile_url": person.profile_url,
            "profile_company": person.profile_company,
            "affiliation": person.affiliation,
            "affiliations": person.affiliations,
        });
        contributor["affiliation_history"] =
            json!(profile_affiliations(&person.affiliation, &person.affiliations, &companies, &format_date(today)));
        Some(json!({
            "contributor": contributor,
            "summary": {
                "pull_requests_opened": totals.get(MetricKind::PullRequestsCreated),
                "pull_requests_merged": totals.get(MetricKind::PullRequestsMerged),
                "commits_authored": if include_history { json!(authored) } else { Value::Null },
                "commits_committed": if include_history { json!(committed) } else { Value::Null },
                "shared_additions": totals.get(MetricKind::SharedAdditions),
                "shared_deletions": totals.get(MetricKind::SharedDeletions),
                "reviews": totals.get(MetricKind::Reviews),
            },
            "daily": daily,
            "filters": {"days": days_value(days), "repository": data.repository_filter_value(repository)},
            "repository_options": options.iter().map(|r| json!({"id": data.repo_path(*r), "name": data.repo(*r).full_name()})).collect::<Vec<_>>(),
            "repository_activity": repository_activity,
            "languages": languages,
            "ai_usage": ai_usage,
            "ai_provider_lines": ai_provider_lines,
            "pull_requests": pull_requests,
            "commits": commits,
            "history_loaded": include_history,
        }))
    }

    /// Explicit AI identities and attribution in one contributor's commits and PRs.
    fn contributor_ai_usage(&self, key: u32, start: i32, selected: Option<u32>) -> Value {
        let data = self.data;
        let today = self.today;
        let selected_ok = |repo: u32| selected.is_none_or(|s| s == repo);
        let mut models: Ordered<String, Identity> = Ordered::new();
        let mut categories: [HashSet<u32>; 3] = Default::default();
        let mut attributed_commits = HashSet::new();
        let mut attributed_prs = HashSet::new();
        let mut record = |role: AiRole, identity: u32, commit: Option<u32>, pr: Option<u32>| {
            if identity == NONE {
                return;
            }
            let model = &data.snapshot.ai_identities[identity as usize];
            let activity = models.entry(model.key.clone());
            activity.describe(model);
            if let Some(commit) = commit {
                activity.commits.insert(commit);
                activity.categories[role.index()].insert(commit);
                attributed_commits.insert(commit);
                categories[role.index()].insert(commit);
            }
            if let Some(pr) = pr {
                activity.pull_requests.insert(pr);
                attributed_prs.insert(pr);
            }
        };

        for (index, commit) in data.snapshot.commits.iter().enumerate() {
            if !selected_ok(commit.repo)
                || (commit.author != key && commit.committer != key)
                || !in_range(day_of(commit.committed_at), Some(start), Some(today))
            {
                continue;
            }
            let Some(ai) = &commit.ai else { continue };
            for (role, identity, _) in ai.attributions(commit.author) {
                record(role, identity, Some(index as u32), None);
            }
        }
        let author_is_ai = data.is_ai(key);
        for (index, pr) in data.snapshot.pull_requests.iter().enumerate() {
            let index = index as u32;
            if !selected_ok(pr.repo) || pr.author != key || !in_range(day_of(pr.created_at), Some(start), Some(today)) {
                continue;
            }
            for &commit in &data.commits_by_pr[index as usize] {
                let commit = &data.snapshot.commits[commit as usize];
                if !selected_ok(commit.repo) {
                    continue;
                }
                let Some(ai) = &commit.ai else { continue };
                for (role, identity, _) in ai.attributions(commit.author) {
                    record(role, identity, None, Some(index));
                }
            }
            if author_is_ai {
                record(AiRole::Author, pr.author_ai_identity, None, Some(index));
            }
        }
        let mut top_models: Vec<Value> = models
            .entries
            .iter()
            .filter(|(_, a)| !a.commits.is_empty() || !a.pull_requests.is_empty())
            .map(|(_, a)| {
                json!({
                    "name": a.name,
                    "provider": a.provider,
                    "model": a.model,
                    "commits": a.commits.len(),
                    "pull_requests": a.pull_requests.len(),
                    "commits_ai_authored": a.categories[AiRole::Author.index()].len(),
                    "commits_ai_coauthored": a.categories[AiRole::CoAuthor.index()].len(),
                    "commits_ai_assisted": a.categories[AiRole::Assistant.index()].len(),
                })
            })
            .collect();
        top_models.sort_by_cached_key(|v| {
            let commits = v["commits"].as_i64().unwrap_or(0);
            std::cmp::Reverse((
                commits + v["pull_requests"].as_i64().unwrap_or(0),
                commits,
                fold(v["name"].as_str().unwrap_or_default()),
            ))
        });
        json!({
            "summary": {
                "commits_with_ai": attributed_commits.len(),
                "commits_ai_authored": categories[AiRole::Author.index()].len(),
                "commits_ai_coauthored": categories[AiRole::CoAuthor.index()].len(),
                "commits_ai_assisted": categories[AiRole::Assistant.index()].len(),
                "pull_requests_with_ai": attributed_prs.len(),
            },
            "top_models": top_models,
        })
    }

    /// Merged PR additions counted once per AI provider for one contributor.
    fn contributor_ai_provider_lines(&self, key: u32, start: i32, selected: Option<u32>) -> Value {
        let data = self.data;
        let provider = |identity: u32| data.snapshot.ai_identities[identity as usize].provider.as_str();
        let mut additions: HashMap<String, i64> = HashMap::new();
        let mut labels: HashMap<String, String> = HashMap::new();
        for (index, pr) in data.snapshot.pull_requests.iter().enumerate() {
            if selected.is_some_and(|s| s != pr.repo)
                || pr.merged_at.is_none()
                || !in_range(day_of(pr.merged_at), Some(start), Some(self.today))
            {
                continue;
            }
            let mut participates = pr.author == key;
            let mut providers: BTreeSet<&str> = BTreeSet::new();
            if pr.author_provider_identity != NONE {
                providers.insert(provider(pr.author_provider_identity));
            }
            for &commit in &data.commits_by_pr[index] {
                let commit = &data.snapshot.commits[commit as usize];
                if commit.author == key {
                    participates = true;
                }
                let Some(ai) = &commit.ai else { continue };
                if ai.author_provider_identity != NONE {
                    providers.insert(provider(ai.author_provider_identity));
                }
                for trailer in &ai.trailers {
                    if trailer.ai_identity != NONE {
                        providers.insert(provider(trailer.ai_identity));
                    } else if trailer.co_authored && trailer.resolved == key {
                        participates = true;
                    }
                }
            }
            if !participates || pr.additions <= 0 {
                continue;
            }
            let category = match providers.len() {
                0 => "No provider identified".to_string(),
                1 => providers.into_iter().next().unwrap_or_default().to_string(),
                _ => "Multiple providers".to_string(),
            };
            let folded = fold(&category);
            labels.insert(folded.clone(), category);
            *additions.entry(folded).or_default() += pr.additions;
        }
        let mut keys: Vec<&String> = additions.keys().collect();
        keys.sort_by(|a, b| (-additions[*a], &labels[*a]).cmp(&(-additions[*b], &labels[*b])));
        json!(keys.into_iter().map(|k| json!({"name": labels[k], "additions": additions[k]})).collect::<Vec<_>>())
    }
}
