//! AI activity overview and AI provider/model detail.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::{Value, json};

use super::activity::sort_by_score;
use super::{Ordered, View, day_of, fold, in_range, normalized_ai_name};
use crate::AI_NAME_ALIASES;
use crate::snapshot::{AiIdentity, AiRole, MetricKind, NONE};
use crate::time::format_date;

#[derive(Default)]
pub(crate) struct AiQuery<'a> {
    pub start: Option<i32>,
    pub end: Option<i32>,
    pub repository: Option<&'a str>,
    pub target_branch: Option<&'a str>,
    pub identity_key: Option<String>,
    pub provider: String,
    pub model: String,
    pub name: String,
    /// `None` returns every AI identity.
    pub top_limit: Option<usize>,
}

/// Labels per [`AiRole`], in `AiRole::ALL` order.
const CATEGORY_LABELS: [&str; 3] = ["AI author", "AI co-author", "AI assisted"];

fn label(role: AiRole) -> &'static str {
    CATEGORY_LABELS[role.index()]
}

/// The flag a pull request gets from a commit with this AI role.
fn commit_flag(role: AiRole) -> &'static str {
    match role {
        AiRole::Author => "ai_commit_author",
        AiRole::CoAuthor => "ai_coauthored",
        AiRole::Assistant => "ai_assisted",
    }
}

/// One AI identity's activity, keyed by the identity key.
#[derive(Default)]
pub(super) struct Identity {
    pub ids: BTreeSet<u32>,
    pub logins: BTreeSet<String>,
    pub name: String,
    pub provider: String,
    pub model: String,
    /// Exact versions seen under this key, such as `4.5` and `4.6`.
    pub versions: BTreeSet<String>,
    pub commits: HashSet<u32>,
    /// Commits per [`AiRole`].
    pub categories: [HashSet<u32>; 3],
    pub pull_requests: HashSet<u32>,
    pub shared_additions: i64,
    pub shared_deletions: i64,
}

impl Identity {
    /// Take the name, provider, and model from the first identity seen for the key,
    /// and record the exact version this one names.
    pub fn describe(&mut self, model: &AiIdentity) {
        if self.name.is_empty() && self.provider.is_empty() {
            self.name = model.name.clone();
            self.provider = model.provider.clone();
            self.model = model.model.clone();
        }
        if !model.version.is_empty() {
            self.versions.insert(model.version.clone());
        }
    }

    fn absorb(&mut self, other: &Identity) {
        self.commits.extend(&other.commits);
        self.pull_requests.extend(&other.pull_requests);
        for (mine, theirs) in self.categories.iter_mut().zip(&other.categories) {
            mine.extend(theirs);
        }
    }
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
    roles: Vec<AiRole>,
    identity_keys: BTreeSet<String>,
    identity_categories: HashMap<String, BTreeSet<AiRole>>,
}

struct PrRow {
    index: u32,
    flags: BTreeSet<&'static str>,
    identity_keys: BTreeSet<String>,
    identity_categories: HashMap<String, BTreeSet<AiRole>>,
}

impl View<'_> {
    pub(crate) fn ai_usage(&self, query: &AiQuery) -> Value {
        let data = self.data;
        let snapshot = &data.snapshot;
        let repo_filter = query.repository.map(|id| data.repo_index(id).unwrap_or(NONE));
        let branch_filter = query.target_branch.map(|b| data.branch_filter(b));
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
            let key = if model.key.is_empty() {
                format!("id:{}", data.person(contributor).map_or("", |p| p.id.as_str()))
            } else {
                model.key.clone()
            };
            let entry = activity.entry(key.clone());
            entry.describe(model);
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
            for (_, identity, contributor) in ai.attributions(commit.author) {
                ensure(&mut activity, identity, contributor);
            }
            let mut identity_keys = BTreeSet::new();
            let mut identity_categories: HashMap<String, BTreeSet<AiRole>> = HashMap::new();
            if commit_in_scope(index) {
                for (role, identity, contributor) in ai.attributions(commit.author) {
                    let key = ensure(&mut activity, identity, contributor);
                    let entry = activity.entry(key.clone());
                    entry.commits.insert(commit_index);
                    entry.categories[role.index()].insert(commit_index);
                    identity_keys.insert(key.clone());
                    identity_categories.entry(key).or_default().insert(role);
                }
            }
            if ai.has_coauthor_trailer {
                totals.coauthor_trailer.insert(commit_index);
            }
            if ai.has_assisted_trailer {
                totals.assisted_trailer.insert(commit_index);
            }
            let roles: Vec<AiRole> = ai.roles().collect();
            for role in &roles {
                totals.commits[role.index()].insert(commit_index);
            }
            if roles.is_empty() {
                continue;
            }
            totals.commits_with_ai.insert(commit_index);
            commit_rows.push(CommitRow { index: commit_index, roles, identity_keys, identity_categories });
        }
        let commit_keys: HashMap<u32, &CommitRow> = commit_rows.iter().map(|r| (r.index, r)).collect();

        // PR flags from the AI attribution of their commits.
        let mut pr_flags: HashMap<u32, BTreeSet<&'static str>> = HashMap::new();
        let mut pr_identities: HashMap<u32, BTreeSet<String>> = HashMap::new();
        let mut pr_categories: HashMap<u32, HashMap<String, BTreeSet<AiRole>>> = HashMap::new();
        for &(pr, commit) in &snapshot.pr_commits {
            if let Some(ai) = &snapshot.commits[commit as usize].ai {
                pr_flags.entry(pr).or_default().extend(ai.roles().map(commit_flag));
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
            if data.is_ai(pr.author) {
                flags.insert("ai_author");
                if pr_in_scope(index) && pr.author_ai_identity != NONE {
                    let key = ensure(&mut activity, pr.author_ai_identity, pr.author);
                    activity.entry(key.clone()).pull_requests.insert(index);
                    pr_identities.entry(index).or_default().insert(key.clone());
                    pr_categories.entry(index).or_default().entry(key).or_default().insert(AiRole::Author);
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
                for role in &row.roles {
                    entry[role.index()].insert(row.index);
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
                    "id": data.repo_path(*repo),
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
            let id = data.key_path(row.key);
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
                "id": if identity.ids.len() == 1 { data.key_path(*identity.ids.iter().next().unwrap_or(&NONE)) } else { "" },
                "login": if identity.logins.len() == 1 { identity.logins.iter().next().cloned().unwrap_or_default() } else { String::new() },
                "name": identity.name,
                "provider": identity.provider,
                "model": identity.model,
                "versions": identity.versions,
                "commits_with_ai": identity.commits.len(),
                "commits_ai_authored": identity.categories[AiRole::Author.index()].len(),
                "commits_ai_coauthored": identity.categories[AiRole::CoAuthor.index()].len(),
                "commits_ai_assisted": identity.categories[AiRole::Assistant.index()].len(),
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
        top_ais.sort_by_cached_key(|v| std::cmp::Reverse((score(v), fold(v["name"].as_str().unwrap_or_default()))));

        let mut providers: Ordered<String, Identity> = Ordered::new();
        for (_, identity) in &activity.entries {
            let entry = providers.entry(identity.provider.clone());
            entry.provider = identity.provider.clone();
            entry.absorb(identity);
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
                    "commits_ai_authored": p.categories[AiRole::Author.index()].len(),
                    "commits_ai_coauthored": p.categories[AiRole::CoAuthor.index()].len(),
                    "commits_ai_assisted": p.categories[AiRole::Assistant.index()].len(),
                    "pull_requests_with_ai": p.pull_requests.len(),
                    "shared_additions": shared.0,
                    "shared_deletions": shared.1,
                })
            })
            .collect();
        top_ai_providers
            .sort_by_cached_key(|v| std::cmp::Reverse((score(v), fold(v["provider"].as_str().unwrap_or_default()))));

        let ai_names = |names: &[crate::snapshot::AiName], selected: Option<&HashSet<String>>| -> Vec<Value> {
            names
                .iter()
                .filter(|n| selected.is_none_or(|s| s.contains(&snapshot.ai_identities[n.identity as usize].key)))
                .map(|n| json!({"id": data.key_path(n.contributor), "name": n.name}))
                .collect()
        };
        let commit_value = |row: &CommitRow, categories: Vec<&str>, selected: Option<&HashSet<String>>| -> Value {
            let commit = &snapshot.commits[row.index as usize];
            let ai = commit.ai.as_deref();
            let mut value = data.commit_view(row.index);
            value["subject"] = json!(commit.subject);
            value["author"] = json!(data.name_or(commit.author, "Unknown"));
            value["categories"] = json!(categories);
            value["ai_coauthors"] = json!(ai.map(|a| ai_names(&a.coauthors, selected)).unwrap_or_default());
            value["ai_assistants"] = json!(ai.map(|a| ai_names(&a.assistants, selected)).unwrap_or_default());
            value
        };
        let pr_value = |row: &PrRow, categories: Value| -> Value {
            let mut value = data.pull_request_view(row.index);
            value["categories"] = categories;
            value
        };
        let labels =
            |categories: BTreeSet<AiRole>| -> Vec<&'static str> { categories.into_iter().map(label).collect() };

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
                aggregate.absorb(identity);
            }
            let provider_name = if !query.provider.is_empty() {
                query.provider.clone()
            } else {
                selected_identities.first().map_or("Other".to_string(), |i| i.provider.clone())
            };
            let categories_for = |map: &HashMap<String, BTreeSet<AiRole>>| -> BTreeSet<AiRole> {
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
                    json!({"date": format_date(day), "pull_requests_merged": prs, "commits": commits})
                })
                .collect();

            let mut repo_activity: Ordered<(u32, u32), [HashSet<u32>; 8]> = Ordered::new();
            for row in &identity_commits {
                let commit = &snapshot.commits[row.index as usize];
                for branch in branch_names_for(commit.repo, row.index) {
                    let entry = repo_activity.entry((commit.repo, branch));
                    entry[0].insert(row.index);
                    for category in categories_for(&row.identity_categories) {
                        entry[1 + category.index()].insert(row.index);
                    }
                }
            }
            for row in &identity_prs {
                let pr = &snapshot.pull_requests[row.index as usize];
                let entry = repo_activity.entry((pr.repo, pr_branch(pr.repo, pr.branch)));
                entry[4].insert(row.index);
                for category in categories_for(&row.identity_categories) {
                    entry[5 + category.index()].insert(row.index);
                }
            }
            let mut repository_activity: Vec<Value> = repo_activity
                .entries
                .iter()
                .map(|((repo, branch), sets)| {
                    let label = branch_label(*repo, *branch);
                    json!({
                        "id": data.repo_path(*repo),
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
                    "commits_ai_authored": aggregate.categories[AiRole::Author.index()].len(),
                    "commits_ai_coauthored": aggregate.categories[AiRole::CoAuthor.index()].len(),
                    "commits_ai_assisted": aggregate.categories[AiRole::Assistant.index()].len(),
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
                "repository_options": data.repository_options(),
                "filters": {"days": filter_days, "repository": data.repository_filter_value(query.repository)},
            })
        });

        if let Some(limit) = query.top_limit {
            top_ais.truncate(limit);
        }
        json!({
            "summary": {
                "commits_with_ai": totals.commits_with_ai.len(),
                "commits_ai_authored": totals.commits[AiRole::Author.index()].len(),
                "commits_ai_coauthored": totals.commits[AiRole::CoAuthor.index()].len(),
                "commits_ai_assisted": totals.commits[AiRole::Assistant.index()].len(),
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
                .map(|r| commit_value(r, r.roles.iter().copied().map(label).collect(), None))
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
