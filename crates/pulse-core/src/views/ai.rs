//! AI activity overview and AI provider/model detail.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::{Value, json};

use super::activity::sort_by_score;
use super::{Ordered, View, date_str, day_of, fold, in_range, iso, normalized_ai_name};
use crate::AI_NAME_ALIASES;
use crate::snapshot::{MetricKind, NONE};

#[derive(Default)]
pub(crate) struct AiQuery<'a> {
    pub start: Option<i32>,
    pub end: Option<i32>,
    pub repository_id: Option<&'a str>,
    pub target_branch: Option<&'a str>,
    pub identity_key: Option<String>,
    pub provider: String,
    pub model: String,
    pub name: String,
    /// `None` returns every AI identity.
    pub top_limit: Option<usize>,
}

const AUTHORED: usize = 0;
const COAUTHORED: usize = 1;
const ASSISTED: usize = 2;
const CATEGORY_LABELS: [&str; 3] = ["AI author", "AI co-author", "AI assisted"];

#[derive(Default)]
struct Identity {
    ids: BTreeSet<u32>,
    logins: BTreeSet<String>,
    name: String,
    provider: String,
    model: String,
    commits: HashSet<u32>,
    categories: [HashSet<u32>; 3],
    pull_requests: HashSet<u32>,
    shared_additions: i64,
    shared_deletions: i64,
}

#[derive(Default)]
struct Totals {
    commits_with_ai: HashSet<u32>,
    commits: [HashSet<u32>; 3],
    coauthor_trailer: HashSet<u32>,
    assisted_trailer: HashSet<u32>,
    pull_requests_with_ai: HashSet<u32>,
    pr_ai_author: HashSet<u32>,
    pr_ai_commit_author: HashSet<u32>,
    pr_ai_coauthored: HashSet<u32>,
    pr_ai_assisted: HashSet<u32>,
}

struct CommitRow {
    index: u32,
    categories: Vec<&'static str>,
    identity_keys: BTreeSet<String>,
    identity_categories: HashMap<String, BTreeSet<usize>>,
}

struct PrRow {
    index: u32,
    flags: BTreeSet<&'static str>,
    identity_keys: BTreeSet<String>,
    identity_categories: HashMap<String, BTreeSet<usize>>,
}

impl View<'_> {
    pub(crate) fn ai_usage(&self, query: &AiQuery) -> Value {
        let data = self.data;
        let snapshot = &data.snapshot;
        let repo_filter = query.repository_id.map(|id| data.repo_index(id).unwrap_or(NONE));
        let branch_filter = query.target_branch.map(|b| data.branch_index(b).unwrap_or(NONE - 1));
        let commit_in_scope = |index: usize| {
            let commit = &snapshot.commits[index];
            repo_filter.is_none_or(|r| r == commit.repo)
                && branch_filter.is_none_or(|b| data.commit_branches[index].contains(&b))
                && in_range(day_of(commit.committed_at), query.start, query.end)
        };
        let pr_in_scope = |index: u32| {
            let pr = &snapshot.pull_requests[index as usize];
            repo_filter.is_none_or(|r| r == pr.repo)
                && branch_filter.is_none_or(|b| pr.branch == b)
                && in_range(day_of(pr.created_at), query.start, query.end)
        };

        let mut activity: Ordered<String, Identity> = Ordered::new();
        let ensure = |activity: &mut Ordered<String, Identity>, identity: u32, contributor: u32| -> String {
            let model = &snapshot.ai_identities[identity as usize];
            let key = if model.key.is_empty() { format!("id:{}", data.key_id(contributor)) } else { model.key.clone() };
            let entry = activity.entry(key.clone());
            if entry.name.is_empty() && entry.provider.is_empty() {
                entry.name = model.name.clone();
                entry.provider = model.provider.clone();
                entry.model = model.model.clone();
            }
            if contributor != NONE {
                entry.ids.insert(contributor);
                let login = data.login(contributor);
                if !login.is_empty() {
                    entry.logins.insert(login);
                }
            }
            key
        };

        let mut totals = Totals::default();
        let mut commit_rows: Vec<CommitRow> = Vec::new();
        for (index, commit) in snapshot.commits.iter().enumerate() {
            let Some(ai) = &commit.ai else { continue };
            let commit_index = index as u32;
            if ai.ai_author {
                ensure(&mut activity, ai.ai_author_identity, commit.author);
            }
            for name in ai.coauthors.iter().chain(&ai.assistants) {
                ensure(&mut activity, name.identity, name.contributor);
            }
            let mut identity_keys = BTreeSet::new();
            let mut identity_categories: HashMap<String, BTreeSet<usize>> = HashMap::new();
            if commit_in_scope(index) {
                let mut record =
                    |activity: &mut Ordered<String, Identity>, identity: u32, contributor: u32, category: usize| {
                        let key = ensure(activity, identity, contributor);
                        let entry = activity.entry(key.clone());
                        entry.commits.insert(commit_index);
                        entry.categories[category].insert(commit_index);
                        identity_keys.insert(key.clone());
                        identity_categories.entry(key).or_default().insert(category);
                    };
                if ai.ai_author {
                    record(&mut activity, ai.ai_author_identity, commit.author, AUTHORED);
                }
                for name in &ai.coauthors {
                    record(&mut activity, name.identity, name.contributor, COAUTHORED);
                }
                for name in &ai.assistants {
                    record(&mut activity, name.identity, name.contributor, ASSISTED);
                }
            }
            if ai.has_coauthor_trailer {
                totals.coauthor_trailer.insert(commit_index);
            }
            if ai.has_assisted_trailer {
                totals.assisted_trailer.insert(commit_index);
            }
            let mut categories = Vec::new();
            for (flag, category, label) in [
                (ai.ai_author, AUTHORED, CATEGORY_LABELS[AUTHORED]),
                (!ai.coauthors.is_empty(), COAUTHORED, CATEGORY_LABELS[COAUTHORED]),
                (!ai.assistants.is_empty(), ASSISTED, CATEGORY_LABELS[ASSISTED]),
            ] {
                if flag {
                    categories.push(label);
                    totals.commits[category].insert(commit_index);
                }
            }
            if categories.is_empty() {
                continue;
            }
            totals.commits_with_ai.insert(commit_index);
            commit_rows.push(CommitRow { index: commit_index, categories, identity_keys, identity_categories });
        }
        let commit_keys: HashMap<u32, &CommitRow> = commit_rows.iter().map(|r| (r.index, r)).collect();

        // PR flags from the AI attribution of their commits.
        let mut pr_flags: HashMap<u32, BTreeSet<&'static str>> = HashMap::new();
        let mut pr_identities: HashMap<u32, BTreeSet<String>> = HashMap::new();
        let mut pr_categories: HashMap<u32, HashMap<String, BTreeSet<usize>>> = HashMap::new();
        for &(pr, commit) in &snapshot.pr_commits {
            if let Some(ai) = &snapshot.commits[commit as usize].ai {
                let flags = pr_flags.entry(pr).or_default();
                if ai.ai_author {
                    flags.insert("ai_commit_author");
                }
                if !ai.coauthors.is_empty() {
                    flags.insert("ai_coauthored");
                }
                if !ai.assistants.is_empty() {
                    flags.insert("ai_assisted");
                }
            }
            if !pr_in_scope(pr) {
                continue;
            }
            if let Some(row) = commit_keys.get(&commit) {
                for key in &row.identity_keys {
                    activity.entry(key.clone()).pull_requests.insert(pr);
                    pr_identities.entry(pr).or_default().insert(key.clone());
                    if let Some(values) = row.identity_categories.get(key) {
                        pr_categories.entry(pr).or_default().entry(key.clone()).or_default().extend(values);
                    }
                }
            }
        }
        let mut pr_rows: Vec<PrRow> = Vec::new();
        for &index in &data.prs_newest_first {
            let pr = &snapshot.pull_requests[index as usize];
            let mut flags = pr_flags.get(&index).cloned().unwrap_or_default();
            if data.kind(pr.author).eq_ignore_ascii_case("AI") {
                flags.insert("ai_author");
                if pr_in_scope(index) && pr.author_ai_identity != NONE {
                    let key = ensure(&mut activity, pr.author_ai_identity, pr.author);
                    activity.entry(key.clone()).pull_requests.insert(index);
                    pr_identities.entry(index).or_default().insert(key.clone());
                    pr_categories.entry(index).or_default().entry(key).or_default().insert(AUTHORED);
                }
            }
            for flag in &flags {
                match *flag {
                    "ai_author" => totals.pr_ai_author.insert(index),
                    "ai_commit_author" => totals.pr_ai_commit_author.insert(index),
                    "ai_coauthored" => totals.pr_ai_coauthored.insert(index),
                    _ => totals.pr_ai_assisted.insert(index),
                };
            }
            if flags.is_empty() {
                continue;
            }
            totals.pull_requests_with_ai.insert(index);
            pr_rows.push(PrRow {
                index,
                flags,
                identity_keys: pr_identities.get(&index).cloned().unwrap_or_default(),
                identity_categories: pr_categories.get(&index).cloned().unwrap_or_default(),
            });
        }

        // AI activity by repository and branch, across all AI-attributed commits and PRs.
        let branch_names_for = |repo: u32, commit: u32| -> Vec<u32> {
            let configured = &data.repo(repo).target_branches;
            if configured.len() <= 1 {
                return vec![0];
            }
            let branches: Vec<u32> = data.commit_branches[commit as usize]
                .iter()
                .copied()
                .filter(|b| configured.iter().any(|c| c.name == data.branch(*b)))
                .collect();
            if branches.is_empty() { vec![0] } else { branches }
        };
        let pr_branch = |repo: u32, branch: u32| if data.repo(repo).target_branches.len() > 1 { branch } else { 0 };
        let mut by_branch: Ordered<(u32, u32), [HashSet<u32>; 7]> = Ordered::new();
        for row in &commit_rows {
            let commit = &snapshot.commits[row.index as usize];
            for branch in branch_names_for(commit.repo, row.index) {
                let entry = by_branch.entry((commit.repo, branch));
                for label in &row.categories {
                    let slot = CATEGORY_LABELS.iter().position(|l| l == label).unwrap_or(0);
                    entry[slot].insert(row.index);
                }
            }
        }
        for row in &pr_rows {
            let pr = &snapshot.pull_requests[row.index as usize];
            let entry = by_branch.entry((pr.repo, pr_branch(pr.repo, pr.branch)));
            for flag in &row.flags {
                let slot = match *flag {
                    "ai_author" => 3,
                    "ai_commit_author" => 4,
                    "ai_coauthored" => 5,
                    _ => 6,
                };
                entry[slot].insert(row.index);
            }
        }
        let branch_label = |repo: u32, branch: u32| -> Option<(String, String)> {
            data.repo(repo)
                .target_branches
                .iter()
                .find(|b| b.name == data.branch(branch) && branch != 0)
                .map(|b| (b.name.clone(), b.label.clone()))
        };
        let mut by_repository: Vec<Value> = by_branch
            .entries
            .iter()
            .map(|((repo, branch), sets)| {
                let label = branch_label(*repo, *branch);
                json!({
                    "id": data.repo(*repo).id,
                    "name": data.repository_label(*repo, label.as_ref().map(|(n, l)| (n.as_str(), l.as_str()))),
                    "target_branch": data.branch(*branch),
                    "commits_ai_authored": sets[0].len(),
                    "commits_ai_coauthored": sets[1].len(),
                    "commits_ai_assisted": sets[2].len(),
                    "pull_requests_ai_authored": sets[3].len(),
                    "pull_requests_with_ai_authored_commit": sets[4].len(),
                    "pull_requests_ai_coauthored": sets[5].len(),
                    "pull_requests_ai_assisted": sets[6].len(),
                })
            })
            .collect();
        sort_by_score(&mut by_repository, |v| {
            [
                "commits_ai_authored",
                "commits_ai_coauthored",
                "commits_ai_assisted",
                "pull_requests_ai_authored",
                "pull_requests_with_ai_authored_commit",
                "pull_requests_ai_coauthored",
                "pull_requests_ai_assisted",
            ]
            .iter()
            .map(|k| v[*k].as_i64().unwrap_or(0))
            .sum()
        });

        // Newest first; stable for equal timestamps.
        commit_rows.sort_by(|a, b| {
            let at = |r: &CommitRow| snapshot.commits[r.index as usize].committed_at.unwrap_or(i64::MIN);
            at(b).cmp(&at(a))
        });

        let mut shared_by_identity: HashMap<String, (i64, i64)> = HashMap::new();
        let mut shared_by_provider: HashMap<String, (i64, i64)> = HashMap::new();
        for row in &snapshot.metrics {
            let additions = match row.kind {
                MetricKind::SharedAdditions => true,
                MetricKind::SharedDeletions => false,
                _ => continue,
            };
            let date_ok = in_range(Some(row.date), query.start, query.end);
            let branch_ok = match branch_filter {
                Some(b) => row.branch == b,
                None => row.branch == 0,
            };
            if !date_ok || !branch_ok || repo_filter.is_some_and(|r| r != row.repo) {
                continue;
            }
            let id = data.key_id(row.key);
            let target = if let Some(key) = id.strip_prefix("ai-model:") {
                shared_by_identity.entry(key.to_string()).or_default()
            } else if let Some(key) = id.strip_prefix("ai-provider:") {
                shared_by_provider.entry(key.to_string()).or_default()
            } else {
                continue;
            };
            if additions {
                target.0 += row.value;
            } else {
                target.1 += row.value;
            }
        }
        for (key, (additions, deletions)) in &shared_by_identity {
            if activity.contains(key) {
                let entry = activity.entry(key.clone());
                entry.shared_additions += additions;
                entry.shared_deletions += deletions;
            }
        }

        let provider_key = |provider: &str| format!("provider:{}", normalized_ai_name(provider));
        let identity_value = |key: &str, identity: &Identity| -> Value {
            json!({
                "key": key,
                "provider_key": provider_key(&identity.provider),
                "id": if identity.ids.len() == 1 { data.key_id(*identity.ids.iter().next().unwrap_or(&NONE)) } else { "" },
                "login": if identity.logins.len() == 1 { identity.logins.iter().next().cloned().unwrap_or_default() } else { String::new() },
                "name": identity.name,
                "provider": identity.provider,
                "model": identity.model,
                "commits_with_ai": identity.commits.len(),
                "commits_ai_authored": identity.categories[AUTHORED].len(),
                "commits_ai_coauthored": identity.categories[COAUTHORED].len(),
                "commits_ai_assisted": identity.categories[ASSISTED].len(),
                "pull_requests_with_ai": identity.pull_requests.len(),
                "shared_additions": identity.shared_additions,
                "shared_deletions": identity.shared_deletions,
            })
        };
        let score = |v: &Value| {
            let commits = v["commits_with_ai"].as_i64().unwrap_or(0);
            (commits + v["pull_requests_with_ai"].as_i64().unwrap_or(0), commits)
        };
        let mut top_ais: Vec<Value> = activity
            .entries
            .iter()
            .filter(|(_, i)| {
                !i.commits.is_empty()
                    || !i.pull_requests.is_empty()
                    || i.shared_additions != 0
                    || i.shared_deletions != 0
            })
            .map(|(key, i)| identity_value(key, i))
            .collect();
        top_ais.sort_by(|a, b| {
            (score(b), fold(b["name"].as_str().unwrap_or_default()))
                .cmp(&(score(a), fold(a["name"].as_str().unwrap_or_default())))
        });

        let mut providers: Ordered<String, Identity> = Ordered::new();
        for (_, identity) in &activity.entries {
            let entry = providers.entry(identity.provider.clone());
            entry.provider = identity.provider.clone();
            entry.commits.extend(&identity.commits);
            entry.pull_requests.extend(&identity.pull_requests);
            for category in 0..3 {
                entry.categories[category].extend(&identity.categories[category]);
            }
        }
        let mut top_ai_providers: Vec<Value> = providers
            .entries
            .iter()
            .map(|(provider, p)| {
                let shared = shared_by_provider.get(&normalized_ai_name(provider)).copied().unwrap_or_default();
                json!({
                    "key": provider_key(provider),
                    "provider": provider,
                    "commits_with_ai": p.commits.len(),
                    "commits_ai_authored": p.categories[AUTHORED].len(),
                    "commits_ai_coauthored": p.categories[COAUTHORED].len(),
                    "commits_ai_assisted": p.categories[ASSISTED].len(),
                    "pull_requests_with_ai": p.pull_requests.len(),
                    "shared_additions": shared.0,
                    "shared_deletions": shared.1,
                })
            })
            .collect();
        top_ai_providers.sort_by(|a, b| {
            (score(b), fold(b["provider"].as_str().unwrap_or_default()))
                .cmp(&(score(a), fold(a["provider"].as_str().unwrap_or_default())))
        });

        let ai_names = |names: &[crate::snapshot::AiName], selected: Option<&HashSet<String>>| -> Vec<Value> {
            names
                .iter()
                .filter(|n| selected.is_none_or(|s| s.contains(&snapshot.ai_identities[n.identity as usize].key)))
                .map(|n| json!({"id": data.key_id(n.contributor), "name": n.name}))
                .collect()
        };
        let commit_value = |row: &CommitRow, categories: Vec<&str>, selected: Option<&HashSet<String>>| -> Value {
            let commit = &snapshot.commits[row.index as usize];
            let ai = commit.ai.as_deref();
            let fallback = |name: &str| if name.is_empty() { "Unknown".to_string() } else { name.to_string() };
            json!({
                "sha": commit.sha_hex(),
                "repository_id": data.repo(commit.repo).id,
                "repository": data.repo(commit.repo).full_name(),
                "target_branches": data.commit_branches[row.index as usize].iter().map(|b| data.branch(*b)).collect::<Vec<_>>(),
                "committed_at": iso(commit.committed_at),
                "subject": commit.subject,
                "author_id": data.key_id(commit.author),
                "author": data.name_or(commit.author, "Unknown"),
                "author_name": data.name_or(commit.author, &fallback(&commit.author_name)),
                "author_login": data.login(commit.author),
                "committer_name": data.name_or(commit.committer, &fallback(&commit.committer_name)),
                "committer_id": data.key_id(commit.committer),
                "committer_login": data.login(commit.committer),
                "categories": categories,
                "ai_coauthors": ai.map(|a| ai_names(&a.coauthors, selected)).unwrap_or_default(),
                "ai_assistants": ai.map(|a| ai_names(&a.assistants, selected)).unwrap_or_default(),
                "url": data.commit_url(commit),
            })
        };
        let pr_value = |row: &PrRow, categories: Value| -> Value {
            let mut value = data.pull_request_view(row.index);
            value["categories"] = categories;
            value
        };

        let identity_detail = query.identity_key.as_deref().map(|identity_key| {
            let requested = activity.get(&identity_key.to_string());
            let provider_profile = identity_key.starts_with("provider:") || !query.provider.is_empty();
            let requested_provider = if let Some(provider) = identity_key.strip_prefix("provider:") {
                provider.to_string()
            } else if provider_profile {
                let provider = if query.provider.is_empty() {
                    requested.map(|i| i.provider.as_str()).unwrap_or_default()
                } else {
                    query.provider.as_str()
                };
                normalized_ai_name(provider)
            } else {
                String::new()
            };
            let selected: HashSet<String> = if provider_profile {
                activity
                    .entries
                    .iter()
                    .filter(|(_, i)| normalized_ai_name(&i.provider) == requested_provider)
                    .map(|(k, _)| k.clone())
                    .collect()
            } else if requested.is_some() {
                HashSet::from([identity_key.to_string()])
            } else {
                HashSet::new()
            };
            let selected_identities: Vec<&Identity> =
                activity.entries.iter().filter(|(k, _)| selected.contains(k)).map(|(_, i)| i).collect();
            let mut aggregate = Identity::default();
            for identity in &selected_identities {
                aggregate.commits.extend(&identity.commits);
                aggregate.pull_requests.extend(&identity.pull_requests);
                for category in 0..3 {
                    aggregate.categories[category].extend(&identity.categories[category]);
                }
            }
            let provider_name = if !query.provider.is_empty() {
                query.provider.clone()
            } else {
                selected_identities.first().map_or("Other".to_string(), |i| i.provider.clone())
            };
            let categories_for = |map: &HashMap<String, BTreeSet<usize>>| -> BTreeSet<usize> {
                selected.iter().filter_map(|k| map.get(k)).flatten().copied().collect()
            };
            let identity_commits: Vec<&CommitRow> =
                commit_rows.iter().filter(|r| r.identity_keys.iter().any(|k| selected.contains(k))).collect();
            let identity_prs: Vec<&PrRow> =
                pr_rows.iter().filter(|r| r.identity_keys.iter().any(|k| selected.contains(k))).collect();

            let mut daily_values: HashMap<i32, (i64, i64)> = HashMap::new();
            let mut event_dates = Vec::new();
            for row in &identity_commits {
                if let Some(day) = day_of(snapshot.commits[row.index as usize].committed_at) {
                    event_dates.push(day);
                    daily_values.entry(day).or_default().1 += 1;
                }
            }
            for row in &identity_prs {
                if let Some(day) = day_of(snapshot.pull_requests[row.index as usize].created_at) {
                    event_dates.push(day);
                    daily_values.entry(day).or_default().0 += 1;
                }
            }
            let detail_end = query.end.unwrap_or(self.today);
            let detail_start = query.start.unwrap_or_else(|| event_dates.iter().copied().min().unwrap_or(detail_end));
            let daily: Vec<Value> = (detail_start..=detail_end)
                .map(|day| {
                    let (prs, commits) = daily_values.get(&day).copied().unwrap_or_default();
                    json!({"date": date_str(day), "pull_requests_merged": prs, "commits": commits})
                })
                .collect();

            let mut repo_activity: Ordered<(u32, u32), [HashSet<u32>; 8]> = Ordered::new();
            for row in &identity_commits {
                let commit = &snapshot.commits[row.index as usize];
                for branch in branch_names_for(commit.repo, row.index) {
                    let entry = repo_activity.entry((commit.repo, branch));
                    entry[0].insert(row.index);
                    for category in categories_for(&row.identity_categories) {
                        entry[1 + category].insert(row.index);
                    }
                }
            }
            for row in &identity_prs {
                let pr = &snapshot.pull_requests[row.index as usize];
                let entry = repo_activity.entry((pr.repo, pr_branch(pr.repo, pr.branch)));
                entry[4].insert(row.index);
                for category in categories_for(&row.identity_categories) {
                    entry[5 + category].insert(row.index);
                }
            }
            let mut repository_activity: Vec<Value> = repo_activity
                .entries
                .iter()
                .map(|((repo, branch), sets)| {
                    let label = branch_label(*repo, *branch);
                    json!({
                        "id": data.repo(*repo).id,
                        "name": data.repository_label(*repo, label.as_ref().map(|(n, l)| (n.as_str(), l.as_str()))),
                        "target_branch": data.branch(*branch),
                        "commits": sets[0].len(),
                        "commits_ai_authored": sets[1].len(),
                        "commits_ai_coauthored": sets[2].len(),
                        "commits_ai_assisted": sets[3].len(),
                        "pull_requests_with_ai": sets[4].len(),
                        "pull_requests_ai_authored": sets[5].len(),
                        "pull_requests_ai_coauthored": sets[6].len(),
                        "pull_requests_ai_assisted": sets[7].len(),
                    })
                })
                .collect();
            sort_by_score(&mut repository_activity, |v| {
                v["commits"].as_i64().unwrap_or(0) + v["pull_requests_with_ai"].as_i64().unwrap_or(0)
            });
            let mut repository_options: Vec<Value> =
                snapshot.repositories.iter().map(|r| json!({"id": r.id, "name": r.full_name()})).collect();
            repository_options.sort_by_key(|r| fold(r["name"].as_str().unwrap_or_default()));
            let filter_days = match (query.start, query.end) {
                (Some(start), Some(end)) => json!(end - start + 1),
                _ => json!("all"),
            };
            let mut models: Vec<Value> = activity
                .entries
                .iter()
                .filter(|(k, _)| selected.contains(k))
                .map(|(k, i)| {
                    let mut value = identity_value(k, i);
                    if let Some(object) = value.as_object_mut() {
                        object.remove("id");
                        object.remove("login");
                    }
                    value
                })
                .collect();
            sort_by_score(&mut models, |v| {
                v["commits_with_ai"].as_i64().unwrap_or(0) + v["pull_requests_with_ai"].as_i64().unwrap_or(0)
            });
            let (shared_additions, shared_deletions) = if provider_profile {
                shared_by_provider.get(&requested_provider).copied().unwrap_or_default()
            } else {
                selected.iter().filter_map(|k| shared_by_identity.get(k)).fold((0, 0), |acc, v| (acc.0 + v.0, acc.1 + v.1))
            };
            let first = selected_identities.first();
            let labels = |categories: BTreeSet<usize>| -> Vec<&str> {
                categories.into_iter().map(|c| CATEGORY_LABELS[c]).collect()
            };
            json!({
                "identity": {
                    "key": if provider_profile { format!("provider:{requested_provider}") } else { identity_key.to_string() },
                    "name": if provider_profile {
                        provider_name.clone()
                    } else {
                        first.map(|i| i.name.clone()).filter(|n| !n.is_empty())
                            .or_else(|| Some(query.name.clone()).filter(|n| !n.is_empty()))
                            .unwrap_or_else(|| "AI".into())
                    },
                    "provider": provider_name,
                    "model": if provider_profile {
                        String::new()
                    } else {
                        first.map(|i| i.model.clone()).filter(|m| !m.is_empty()).unwrap_or_else(|| query.model.clone())
                    },
                },
                "summary": {
                    "commits_with_ai": aggregate.commits.len(),
                    "commits_ai_authored": aggregate.categories[AUTHORED].len(),
                    "commits_ai_coauthored": aggregate.categories[COAUTHORED].len(),
                    "commits_ai_assisted": aggregate.categories[ASSISTED].len(),
                    "pull_requests_with_ai": aggregate.pull_requests.len(),
                    "shared_additions": shared_additions,
                    "shared_deletions": shared_deletions,
                },
                "models": models,
                "daily": daily,
                "repository_activity": repository_activity,
                "recent_commits": identity_commits.iter().take(100)
                    .map(|r| commit_value(r, labels(categories_for(&r.identity_categories)), Some(&selected)))
                    .collect::<Vec<_>>(),
                "recent_pull_requests": identity_prs.iter().take(100)
                    .map(|r| pr_value(r, json!(labels(categories_for(&r.identity_categories)))))
                    .collect::<Vec<_>>(),
                "repository_options": repository_options,
                "filters": {"days": filter_days, "repository_id": query.repository_id.unwrap_or("")},
            })
        });

        if let Some(limit) = query.top_limit {
            top_ais.truncate(limit);
        }
        json!({
            "summary": {
                "commits_with_ai": totals.commits_with_ai.len(),
                "commits_ai_authored": totals.commits[AUTHORED].len(),
                "commits_ai_coauthored": totals.commits[COAUTHORED].len(),
                "commits_ai_assisted": totals.commits[ASSISTED].len(),
                "pull_requests_with_ai": totals.pull_requests_with_ai.len(),
                "pull_requests_ai_authored": totals.pr_ai_author.len(),
                "pull_requests_with_ai_authored_commit": totals.pr_ai_commit_author.len(),
                "pull_requests_ai_coauthored": totals.pr_ai_coauthored.len(),
                "pull_requests_ai_assisted": totals.pr_ai_assisted.len(),
                "commits_with_coauthor_trailer": totals.coauthor_trailer.len(),
                "commits_with_assisted_trailer": totals.assisted_trailer.len(),
            },
            "top_ais": top_ais,
            "top_ai_providers": top_ai_providers,
            "by_repository": by_repository,
            "recent_commits": commit_rows.iter().take(100)
                .map(|r| commit_value(r, r.categories.clone(), None))
                .collect::<Vec<_>>(),
            "recent_pull_requests": pr_rows.iter().take(100)
                .map(|r| pr_value(r, json!(r.flags.iter().collect::<Vec<_>>())))
                .collect::<Vec<_>>(),
            "identity_detail": identity_detail,
            "recognition": format!(
                "AI identities use the catalog's AI contributor classification or common tool names in commit trailers. Recognized names: {}. Other trailer names are not assumed to be AI.",
                AI_NAME_ALIASES.join(", ")
            ),
        })
    }
}
