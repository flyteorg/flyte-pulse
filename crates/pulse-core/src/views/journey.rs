//! A contributor's journey (`contributor_journey`): the span of their activity, a
//! dated timeline of firsts and milestones, achievements, and monthly totals across
//! their whole history.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::{Value, json};

use super::company::profile_affiliations;
use super::languages::language_rows;
use super::{Totals, View, day_of};
use crate::snapshot::{Metric, MetricKind, Pr};
use crate::time::{civil_from_days, days_from_civil, format_date};

/// Counts of merged pull requests and commits that mark the timeline.
const COUNT_LADDER: [i64; 7] = [1, 10, 50, 100, 250, 500, 1000];
const REVIEW_LADDER: [i64; 5] = [1, 10, 50, 100, 500];
/// Shared lines added plus removed.
const SHARED_LINES_LADDER: [i64; 2] = [100_000, 1_000_000];
/// A first contribution before this day earns "Early adopter": Flyte's first year in
/// the open.
const EARLY_ADOPTER_BEFORE: (i32, u32, u32) = (2021, 1, 1);
/// A language counts for "Polyglot" from this share of a repository's code.
const LANGUAGE_SHARE: f64 = 0.05;
/// Rank tiers shown on totals, as a fraction of the ranked contributors; worse ranks
/// stay plain.
const RANK_TIERS: [(f64, &str); 3] = [(0.01, "top 1%"), (0.05, "top 5%"), (0.10, "top 10%")];

/// Year and month of a day as `year * 12 + month - 1`.
fn month_of(day: i32) -> i32 {
    let (year, month, _) = civil_from_days(day);
    year * 12 + month as i32 - 1
}

/// `YYYY-MM`
fn month_label(month: i32) -> String {
    format!("{:04}-{:02}", month.div_euclid(12), month.rem_euclid(12) + 1)
}

/// The same month and day `years` later; February 29 becomes March 1.
fn add_years(day: i32, years: i32) -> i32 {
    let (year, month, date) = civil_from_days(day);
    days_from_civil(year + years, month, date)
}

/// The day a running total first reaches each of `targets`, from dated increments in
/// chronological order.
fn crossings(increments: impl IntoIterator<Item = (i32, i64)>, targets: &[i64]) -> Vec<(i64, i32)> {
    let mut out = Vec::new();
    let mut total = 0;
    let mut next = 0;
    for (day, value) in increments {
        total += value;
        while next < targets.len() && total >= targets[next] {
            out.push((targets[next], day));
            next += 1;
        }
    }
    out
}

fn count_label(count: i64, singular: &str, plural: &str) -> String {
    let noun = if count == 1 { singular } else { plural };
    format!("{count} {noun}")
}

/// A duration between two timestamps as a short phrase.
fn duration_label(seconds: i64) -> String {
    match seconds {
        s if s < 60 => count_label(s.max(1), "second", "seconds"),
        s if s < 3600 => count_label(s / 60, "minute", "minutes"),
        s if s < 86_400 => count_label(s / 3600, "hour", "hours"),
        s => count_label(s / 86_400, "day", "days"),
    }
}

struct Event {
    day: i32,
    /// Orders events on the same day.
    order: u8,
    kind: &'static str,
    title: String,
    detail: String,
    url: String,
}

impl Event {
    fn new(day: i32, order: u8, kind: &'static str, title: impl Into<String>) -> Self {
        Self { day, order, kind, title: title.into(), detail: String::new(), url: String::new() }
    }

    fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    fn url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }

    fn value(&self) -> Value {
        json!({
            "date": format_date(self.day),
            "kind": self.kind,
            "title": self.title,
            "detail": self.detail,
            "url": self.url,
        })
    }
}

struct Achievement {
    key: &'static str,
    name: &'static str,
    description: String,
    unlocked_at: Option<i32>,
    /// Progress toward the target, when the achievement is a count.
    progress: Option<(i64, i64)>,
}

impl Achievement {
    fn count(
        key: &'static str,
        name: &'static str,
        noun: &str,
        target: i64,
        value: i64,
        unlocked_at: Option<i32>,
    ) -> Self {
        Self {
            key,
            name,
            description: format!("{target} {noun}"),
            unlocked_at,
            progress: Some((value.min(target), target)),
        }
    }

    fn value(&self) -> Value {
        json!({
            "key": self.key,
            "name": self.name,
            "description": self.description,
            "unlocked": self.unlocked_at.is_some(),
            "unlocked_at": self.unlocked_at.map(format_date),
            "value": self.progress.map(|p| p.0),
            "target": self.progress.map(|p| p.1),
        })
    }
}

/// Sums of the ranked metrics per contributor: merged pull requests, commits,
/// reviews, and shared lines changed.
#[derive(Clone, Copy, Default)]
struct Ranked([i64; 4]);

const RANKED_KEYS: [&str; 4] = ["pull_requests_merged", "commits", "reviews", "shared_lines"];

impl Ranked {
    fn add(&mut self, row: &Metric) {
        match row.kind {
            MetricKind::PullRequestsMerged => self.0[0] += row.value,
            MetricKind::Commits => self.0[1] += row.value,
            MetricKind::Reviews => self.0[2] += row.value,
            MetricKind::SharedAdditions | MetricKind::SharedDeletions => self.0[3] += row.value,
            _ => {}
        }
    }
}

impl View<'_> {
    /// The journey of one contributor, over all recorded time. `None` for an unknown
    /// identifier.
    pub(crate) fn contributor_journey(&self, id: &str) -> Option<Value> {
        let data = self.data;
        let snapshot = &data.snapshot;
        let key = data.key_index(id).filter(|&k| data.person(k).is_some())?;
        let person = data.person(key)?;
        let today = self.today;

        // Repository-wide daily rows, oldest first.
        let mut rows: Vec<&Metric> = data
            .metrics_by_key
            .get(key as usize)
            .into_iter()
            .flatten()
            .map(|&i| &snapshot.metrics[i as usize])
            .filter(|row| row.branch == 0 && row.value != 0 && row.date <= today)
            .collect();
        rows.sort_by_key(|row| row.date);

        let mut totals = Totals::default();
        let mut months: BTreeMap<i32, Totals> = BTreeMap::new();
        let mut active_days: BTreeSet<i32> = BTreeSet::new();
        let mut repo_first: HashMap<u32, i32> = HashMap::new();
        let mut commits_by_repo: HashMap<u32, i64> = HashMap::new();
        for row in &rows {
            totals.add(row.kind, row.value);
            months.entry(month_of(row.date)).or_default().add(row.kind, row.value);
            active_days.insert(row.date);
            if matches!(
                row.kind,
                MetricKind::Commits
                    | MetricKind::PullRequestsCreated
                    | MetricKind::PullRequestsMerged
                    | MetricKind::Reviews
            ) {
                repo_first.entry(row.repo).and_modify(|d| *d = (*d).min(row.date)).or_insert(row.date);
            }
            if row.kind == MetricKind::Commits {
                *commits_by_repo.entry(row.repo).or_default() += row.value;
            }
        }
        let first_day = active_days.first().copied();
        let last_day = active_days.last().copied();

        let mut events: Vec<Event> = Vec::new();
        let mut achievements: Vec<Achievement> = Vec::new();

        // Pull requests, by creation and by merge.
        let pr_detail = |pr: &Pr| format!("#{} {} · {}", pr.number, pr.title, data.repo(pr.repo).full_name());
        let mut prs: Vec<(i32, u32)> = snapshot
            .pull_requests
            .iter()
            .enumerate()
            .filter(|(_, pr)| pr.author == key)
            .filter_map(|(i, pr)| day_of(pr.created_at).filter(|&d| d <= today).map(|d| (d, i as u32)))
            .collect();
        prs.sort_unstable();
        if let Some(&(day, index)) = prs.first() {
            let pr = &snapshot.pull_requests[index as usize];
            events.push(
                Event::new(day, 1, "first", "First pull request opened").detail(pr_detail(pr)).url(data.pr_url(pr)),
            );
        }
        let mut merged: Vec<(i32, u32)> = snapshot
            .pull_requests
            .iter()
            .enumerate()
            .filter(|(_, pr)| pr.author == key)
            .filter_map(|(i, pr)| day_of(pr.merged_at).filter(|&d| d <= today).map(|d| (d, i as u32)))
            .collect();
        merged.sort_unstable();
        for (count, &(day, index)) in merged.iter().enumerate() {
            let count = count as i64 + 1;
            if !COUNT_LADDER.contains(&count) {
                continue;
            }
            let pr = &snapshot.pull_requests[index as usize];
            let title = if count == 1 {
                "First pull request merged".to_string()
            } else {
                format!("{count} merged pull requests")
            };
            events.push(Event::new(day, 2, "milestone", title).detail(pr_detail(pr)).url(data.pr_url(pr)));
        }
        let merged_count = merged.len() as i64;
        let merged_day = |target: usize| merged.get(target - 1).map(|&(day, _)| day);
        for (key, name, target) in [
            ("first_merge", "First merge", 1),
            ("ten", "Ten", 10),
            ("fifty", "Fifty", 50),
            ("century", "Century", 100),
            ("five_hundred", "500 club", 500),
        ] {
            achievements.push(Achievement::count(
                key,
                name,
                "merged pull requests",
                target,
                merged_count,
                merged_day(target as usize),
            ));
        }
        if let Some(&(_, index)) = merged
            .iter()
            .filter(|&&(_, i)| {
                let pr = &snapshot.pull_requests[i as usize];
                pr.additions + pr.deletions > 0
            })
            .max_by_key(|&&(_, i)| {
                let pr = &snapshot.pull_requests[i as usize];
                pr.additions + pr.deletions
            })
        {
            let pr = &snapshot.pull_requests[index as usize];
            let day = day_of(pr.merged_at).unwrap_or(today);
            events.push(
                Event::new(day, 6, "record", "Largest merged pull request")
                    .detail(format!("+{} −{} · {}", pr.additions, pr.deletions, pr_detail(pr)))
                    .url(data.pr_url(pr)),
            );
        }
        if let Some((seconds, index)) = merged
            .iter()
            .filter_map(|&(_, i)| {
                let pr = &snapshot.pull_requests[i as usize];
                Some((pr.merged_at? - pr.created_at?, i)).filter(|(s, _)| *s > 0)
            })
            .min()
        {
            let pr = &snapshot.pull_requests[index as usize];
            let day = day_of(pr.merged_at).unwrap_or(today);
            events.push(
                Event::new(day, 6, "record", format!("Fastest merge: {}", duration_label(seconds)))
                    .detail(pr_detail(pr))
                    .url(data.pr_url(pr)),
            );
        }

        // Commits on target branches.
        let mut commits: Vec<(i32, u32)> = snapshot
            .commits
            .iter()
            .enumerate()
            .filter(|(_, c)| c.author == key)
            .filter_map(|(i, c)| day_of(c.committed_at).filter(|&d| d <= today).map(|d| (d, i as u32)))
            .collect();
        commits.sort_unstable();
        for (count, &(day, index)) in commits.iter().enumerate() {
            let count = count as i64 + 1;
            if !COUNT_LADDER.contains(&count) {
                continue;
            }
            let commit = &snapshot.commits[index as usize];
            let title =
                if count == 1 { "First commit on a target branch".to_string() } else { format!("{count} commits") };
            events.push(
                Event::new(day, 3, "milestone", title)
                    .detail(format!("{} · {}", commit.subject, data.repo(commit.repo).full_name()))
                    .url(data.commit_url(commit)),
            );
        }
        achievements.push(Achievement::count(
            "committer",
            "Committer",
            "commits on target branches",
            100,
            commits.len() as i64,
            commits.get(99).map(|&(day, _)| day),
        ));
        if let Some(&(day, index)) = commits.iter().find(|&&(_, i)| {
            snapshot.commits[i as usize]
                .ai
                .as_ref()
                .is_some_and(|ai| !ai.coauthors.is_empty() || !ai.assistants.is_empty())
        }) {
            let commit = &snapshot.commits[index as usize];
            let names: BTreeSet<&str> = commit
                .ai
                .as_ref()
                .map(|ai| ai.coauthors.iter().chain(&ai.assistants).map(|n| n.name.as_str()).collect())
                .unwrap_or_default();
            let names: Vec<&str> = names.into_iter().collect();
            events.push(
                Event::new(day, 5, "ai", "First AI-assisted commit")
                    .detail(format!("{} · {}", names.join(", "), commit.subject))
                    .url(data.commit_url(commit)),
            );
            achievements.push(Achievement {
                key: "ai_pioneer",
                name: "AI pioneer",
                description: "A commit with AI attribution".into(),
                unlocked_at: Some(day),
                progress: None,
            });
        } else {
            achievements.push(Achievement {
                key: "ai_pioneer",
                name: "AI pioneer",
                description: "A commit with AI attribution".into(),
                unlocked_at: None,
                progress: None,
            });
        }

        // Reviews and shared lines come from the daily rows.
        let series = |kinds: &[MetricKind]| -> Vec<(i32, i64)> {
            rows.iter().filter(|row| kinds.contains(&row.kind)).map(|row| (row.date, row.value)).collect()
        };
        let reviews = crossings(series(&[MetricKind::Reviews]), &REVIEW_LADDER);
        for &(count, day) in &reviews {
            let title = if count == 1 {
                "First pull request reviewed".to_string()
            } else {
                format!("{count} pull requests reviewed")
            };
            events.push(Event::new(day, 4, "milestone", title));
        }
        achievements.push(Achievement::count(
            "reviewer",
            "Reviewer",
            "pull requests reviewed",
            100,
            totals.get(MetricKind::Reviews),
            reviews.iter().find(|(count, _)| *count == 100).map(|&(_, day)| day),
        ));
        let shared_lines = totals.get(MetricKind::SharedAdditions) + totals.get(MetricKind::SharedDeletions);
        for (count, day) in
            crossings(series(&[MetricKind::SharedAdditions, MetricKind::SharedDeletions]), &SHARED_LINES_LADDER)
        {
            events.push(
                Event::new(day, 4, "milestone", format!("{} shared lines changed", count_text(count)))
                    .detail("Lines of merged pull requests, shared with co-authors"),
            );
        }

        // Repositories, in the order they were first contributed to.
        let mut repos: Vec<(i32, u32)> = repo_first.iter().map(|(&repo, &day)| (day, repo)).collect();
        repos.sort_unstable();
        for &(day, repo) in &repos {
            events.push(
                Event::new(day, 0, "repository", format!("First contribution to {}", data.repo(repo).full_name()))
                    .url(format!("#/repositories/{}", data.repo_path(repo))),
            );
        }
        for (key, name, target) in [("repositories_3", "Explorer", 3), ("repositories_5", "Cartographer", 5)] {
            achievements.push(Achievement::count(
                key,
                name,
                "repositories contributed to",
                target,
                repos.len() as i64,
                repos.get(target as usize - 1).map(|&(day, _)| day),
            ));
        }

        // Languages: those with a meaningful share of a repository this person committed to,
        // in the order the repositories were reached.
        let mut languages: BTreeSet<&str> = BTreeSet::new();
        let mut polyglot_day = None;
        for &(day, repo) in &repos {
            if !commits_by_repo.contains_key(&repo) {
                continue;
            }
            let repo_languages = &data.repo(repo).languages;
            let total: u64 = repo_languages.iter().map(|l| l.bytes).sum();
            for language in repo_languages {
                if total > 0 && language.bytes as f64 / total as f64 >= LANGUAGE_SHARE {
                    languages.insert(&language.name);
                }
            }
            if polyglot_day.is_none() && languages.len() >= 3 {
                polyglot_day = Some(day);
            }
        }
        achievements.push(Achievement::count(
            "polyglot",
            "Polyglot",
            "languages",
            3,
            languages.len() as i64,
            polyglot_day,
        ));

        // Affiliations.
        let companies: HashMap<&str, &crate::snapshot::Company> =
            snapshot.companies.iter().map(|c| (c.id.as_str(), c)).collect();
        let affiliation_history =
            profile_affiliations(&person.affiliation, &person.affiliations, &companies, &format_date(today));
        for row in &affiliation_history {
            let name = row["company_name"].as_str().unwrap_or_default();
            let independent = row["company_id"] == "independent";
            if let Some(day) = row["valid_from"].as_str().and_then(crate::time::parse_date).filter(|&d| d <= today) {
                let title = if independent { "Independent contributor".to_string() } else { format!("Joined {name}") };
                events.push(Event::new(day, 7, "affiliation", title));
            }
            if let Some(day) = row["valid_until"].as_str().and_then(crate::time::parse_date).filter(|&d| d <= today)
                && !independent
            {
                events.push(Event::new(day, 7, "affiliation", format!("Left {name}")));
            }
        }

        // Anniversaries and tenure.
        let mut years = 0;
        if let Some(first) = first_day {
            while add_years(first, years + 1) <= today {
                years += 1;
                let day = add_years(first, years);
                events.push(Event::new(
                    day,
                    8,
                    "anniversary",
                    format!("{} of contributing", count_label(years as i64, "year", "years")),
                ));
            }
        }
        achievements.push(Achievement {
            key: "veteran",
            name: "Veteran",
            description: "5 years since the first contribution".into(),
            unlocked_at: first_day.map(|first| add_years(first, 5)).filter(|&d| d <= today),
            progress: Some(((years as i64).min(5), 5)),
        });
        let early_day = days_from_civil(EARLY_ADOPTER_BEFORE.0, EARLY_ADOPTER_BEFORE.1, EARLY_ADOPTER_BEFORE.2);
        achievements.push(Achievement {
            key: "early_adopter",
            name: "Early adopter",
            description: format!("First contribution before {}", format_date(early_day)),
            unlocked_at: first_day.filter(|&d| d < early_day),
            progress: None,
        });

        // Months: a continuous series, the busiest month, and the longest streak.
        let mut monthly = Vec::new();
        let mut busiest: Option<(i64, i32)> = None;
        let mut streak = (0, 0, 0); // length, first month, last month
        let mut longest = streak;
        let mut marathon_day = None;
        if let (Some(&first_month), Some(&last_month)) = (months.keys().next(), months.keys().next_back()) {
            for month in first_month..=last_month {
                let values = months.get(&month).copied().unwrap_or_default();
                let merged = values.get(MetricKind::PullRequestsMerged);
                let commits = values.get(MetricKind::Commits);
                let reviews = values.get(MetricKind::Reviews);
                monthly.push(json!({
                    "date": month_label(month),
                    "pull_requests_merged": merged,
                    "commits": commits,
                    "reviews": reviews,
                    "pull_requests_opened": values.get(MetricKind::PullRequestsCreated),
                }));
                let score = merged + commits + reviews;
                if score > 0 && busiest.is_none_or(|(best, _)| score > best) {
                    busiest = Some((score, month));
                }
                if months.contains_key(&month) {
                    streak = if streak.0 == 0 { (1, month, month) } else { (streak.0 + 1, streak.1, month) };
                    if streak.0 > longest.0 {
                        longest = streak;
                    }
                    if streak.0 == 12 && marathon_day.is_none() {
                        marathon_day = active_days.iter().copied().find(|&d| month_of(d) == month);
                    }
                } else {
                    streak = (0, 0, 0);
                }
            }
        }
        achievements.push(Achievement::count(
            "marathon",
            "Marathon",
            "consecutive active months",
            12,
            longest.0 as i64,
            marathon_day,
        ));

        if let Some(day) = last_day
            && Some(day) != first_day
        {
            events.push(Event::new(day, 9, "latest", "Latest recorded activity"));
        }
        events.sort_by_key(|e| (e.day, e.order));

        Some(json!({
            "contributor": {
                "id": data.key_path(key),
                "name": data.display(key),
                "login": person.login,
                "kind": person.kind,
                "profile_url": person.profile_url,
                "affiliation_history": affiliation_history,
            },
            "span": {
                "first_date": first_day.map(format_date),
                "last_date": last_day.map(format_date),
                "years": years,
                "active_days": active_days.len(),
                "active_months": months.len(),
            },
            "totals": {
                "pull_requests_opened": totals.get(MetricKind::PullRequestsCreated),
                "pull_requests_merged": totals.get(MetricKind::PullRequestsMerged),
                "commits": totals.get(MetricKind::Commits),
                "reviews": totals.get(MetricKind::Reviews),
                "repositories": repos.len(),
                "shared_additions": totals.get(MetricKind::SharedAdditions),
                "shared_deletions": totals.get(MetricKind::SharedDeletions),
                "shared_lines": shared_lines,
                "languages": languages.len(),
            },
            "ranks": self.rank_tiers(key),
            "monthly": monthly,
            "highlights": {
                "busiest_month": busiest.map(|(_, month)| {
                    let values = months[&month];
                    json!({
                        "month": month_label(month),
                        "pull_requests_merged": values.get(MetricKind::PullRequestsMerged),
                        "commits": values.get(MetricKind::Commits),
                        "reviews": values.get(MetricKind::Reviews),
                    })
                }),
                "longest_streak": (longest.0 > 0).then(|| json!({
                    "months": longest.0,
                    "from": month_label(longest.1),
                    "to": month_label(longest.2),
                })),
            },
            "timeline": events.iter().map(Event::value).collect::<Vec<_>>(),
            "achievements": achievements.iter().map(Achievement::value).collect::<Vec<_>>(),
            "languages": language_rows(
                commits_by_repo.iter().map(|(&repo, &commits)| (&data.repo(repo).languages[..], commits as f64)),
            ),
        }))
    }

    /// Where this contributor stands among all humans on each ranked metric, only
    /// for the tiers worth showing: `{metric: {rank, of, tier}}`.
    fn rank_tiers(&self, key: u32) -> Value {
        let data = self.data;
        let mut sums: HashMap<u32, Ranked> = HashMap::new();
        for row in &data.snapshot.metrics {
            if row.branch == 0 && row.date <= self.today && data.is_human(row.key) {
                sums.entry(row.key).or_default().add(row);
            }
        }
        let mine = sums.get(&key).copied().unwrap_or_default();
        let mut out = serde_json::Map::new();
        for (index, name) in RANKED_KEYS.iter().enumerate() {
            let value = mine.0[index];
            if value <= 0 {
                continue;
            }
            let of = sums.values().filter(|r| r.0[index] > 0).count();
            let rank = 1 + sums.values().filter(|r| r.0[index] > value).count();
            let tier = if rank == 1 {
                Some("#1")
            } else {
                RANK_TIERS.iter().find(|(share, _)| rank as f64 / of as f64 <= *share).map(|(_, label)| *label)
            };
            if let Some(tier) = tier {
                out.insert(name.to_string(), json!({"rank": rank, "of": of, "tier": tier}));
            }
        }
        Value::Object(out)
    }
}

/// A count with thousands separators.
fn count_text(value: i64) -> String {
    let digits = value.abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if value < 0 { format!("-{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_and_years() {
        let day = days_from_civil(2024, 2, 29);
        assert_eq!(month_label(month_of(day)), "2024-02");
        assert_eq!(format_date(add_years(day, 1)), "2025-03-01");
        assert_eq!(month_label(month_of(days_from_civil(2023, 12, 31)) + 1), "2024-01");
    }

    #[test]
    fn crossings_report_the_day_a_target_is_reached() {
        let days = crossings([(1, 4), (2, 5), (3, 20), (4, 100)], &[1, 10, 50, 100]);
        assert_eq!(days, [(1, 1), (10, 3), (50, 4), (100, 4)]);
    }

    #[test]
    fn labels() {
        assert_eq!(count_text(1_000_000), "1,000,000");
        assert_eq!(count_text(999), "999");
        assert_eq!(duration_label(45), "45 seconds");
        assert_eq!(duration_label(3 * 3600 + 5), "3 hours");
        assert_eq!(duration_label(3 * 86_400), "3 days");
    }
}
