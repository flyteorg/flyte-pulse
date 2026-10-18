//! Company directory and detail pages.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{Value, json};

use super::{ApiError, ApiResult, Params, View, date_str, days_value, fold, iso, parse_days};
use crate::snapshot::{Affiliation, Company, CompanyMetricKind, NONE};

/// Combine the current mapping and past periods, without inferring dates.
pub(crate) fn profile_affiliations(
    current: &Affiliation,
    periods: &[Affiliation],
    companies: &HashMap<&str, &Company>,
    day: &str,
) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut rows: Vec<(bool, String, Value)> = Vec::new();
    for affiliation in std::iter::once(current).chain(periods) {
        let start = affiliation.valid_from.as_str();
        let end = affiliation.valid_until.as_str();
        if !start.is_empty() && start > day {
            continue;
        }
        let company_id =
            if affiliation.company_id.is_empty() { "independent" } else { affiliation.company_id.as_str() };
        if !seen.insert((company_id.to_string(), start.to_string(), end.to_string())) {
            continue;
        }
        let company = companies.get(company_id);
        let company_name = company
            .map(|c| c.canonical_name.clone())
            .filter(|n| !n.is_empty())
            .or_else(|| Some(affiliation.company_name.clone()).filter(|n| !n.is_empty()))
            .unwrap_or_else(|| if company_id == "independent" { "Independent".into() } else { company_id.into() });
        let website = company
            .map(|c| c.website_url.clone())
            .filter(|w| !w.is_empty())
            .unwrap_or_else(|| affiliation.website_url.clone());
        let is_current = end.is_empty() || day < end;
        let mut value = serde_json::to_value(affiliation).unwrap_or_default();
        value["company_id"] = json!(company_id);
        value["company_name"] = json!(company_name);
        value["website_url"] = json!(website);
        value["is_current"] = json!(is_current);
        rows.push((is_current, start.to_string(), value));
    }
    // sorted(key=(is_current, valid_from), reverse=True), stable.
    rows.sort_by(|a, b| (b.0, &b.1).cmp(&(a.0, &a.1)));
    rows.into_iter().map(|(_, _, v)| v).collect()
}

impl View<'_> {
    pub(crate) fn companies(&self, company_id: &str, params: &Params) -> ApiResult {
        let data = self.data;
        let days = parse_days(params.get_or("days", "365"))?;
        let repository_id = params.get_or("repository_id", "");
        let target_branch = params.get_or("target_branch", "");
        let independent = Company {
            id: "independent".into(),
            canonical_name: "Independent".into(),
            aliases: Vec::new(),
            website_url: String::new(),
        };
        let mut companies: Vec<&Company> = data.snapshot.companies.iter().collect();
        companies.push(&independent);
        let company_index: HashMap<&str, u32> =
            companies.iter().enumerate().map(|(i, c)| (c.id.as_str(), i as u32)).collect();
        let selected = if company_id.is_empty() {
            None
        } else {
            Some(*company_index.get(company_id).ok_or_else(|| ApiError::new(404, "Company not found"))?)
        };
        let today = self.today;
        let start = days.map(|d| today - (d - 1));
        let end = today;

        let mut repository_options = Vec::new();
        let mut repo_names: HashMap<(u32, u32), String> = HashMap::new();
        for (index, repo) in data.snapshot.repositories.iter().enumerate() {
            let index = index as u32;
            let name = format!("{}/{}", repo.owner, repo.name);
            let views: Vec<(&str, &str)> = if repo.target_branches.len() > 1 {
                repo.target_branches.iter().map(|b| (b.name.as_str(), b.label.as_str())).collect()
            } else {
                vec![("", "")]
            };
            for (branch, label) in views {
                let display = if repo.target_branches.len() > 1 {
                    format!("{name} · {}", if label.is_empty() { branch } else { label })
                } else {
                    name.clone()
                };
                let branch_index = data.branch_index(branch).unwrap_or(NONE - 1);
                repo_names.insert((index, branch_index), display.clone());
                repository_options.push(json!({"id": repo.id, "target_branch": branch, "name": display}));
            }
        }
        repository_options.sort_by_key(|r| fold(r["name"].as_str().unwrap_or_default()));

        let repo_filter = (!repository_id.is_empty()).then(|| data.repo_index(repository_id).unwrap_or(NONE));
        let target = data.branch_index(target_branch).unwrap_or(NONE - 1);
        let in_scope = |date: i32, repo: u32| {
            start.is_none_or(|s| date >= s) && date <= end && repo_filter.is_none_or(|r| r == repo)
        };

        let mut totals: HashMap<u32, [i64; 5]> = HashMap::new();
        let mut daily: BTreeMap<i32, [i64; 5]> = BTreeMap::new();
        let mut repository_totals: BTreeMap<(String, String), (u32, u32, [i64; 5])> = BTreeMap::new();
        for row in &data.snapshot.company_metrics {
            if !in_scope(row.date, row.repo) || selected.is_some_and(|s| s != row.company) {
                continue;
            }
            let metric = row.kind as usize;
            if row.branch == target {
                totals.entry(row.company).or_default()[metric] += row.value;
                daily.entry(row.date).or_default()[metric] += row.value;
            }
            // Branch rows split multi-branch repositories without duplicating globals.
            let multi = data.repo(row.repo).target_branches.len() > 1;
            let include = if !target_branch.is_empty() {
                row.branch == target
            } else if multi {
                row.branch != 0
            } else {
                row.branch == 0
            };
            if include {
                let entry = repository_totals
                    .entry((data.repo(row.repo).id.clone(), data.branch(row.branch).to_string()))
                    .or_insert((row.repo, row.branch, [0; 5]));
                entry.2[metric] += row.value;
            }
        }

        let (mut coverage_total, mut coverage_reviewed) = (0i64, 0i64);
        for row in &data.snapshot.company_coverage {
            if in_scope(row.date, row.repo)
                && row.branch == target
                && matches!(row.kind, CompanyMetricKind::Commits | CompanyMetricKind::PullRequestsOpened)
            {
                coverage_total += row.value;
                coverage_reviewed += row.reviewed;
            }
        }

        let company_map: HashMap<&str, &Company> = companies.iter().map(|c| (c.id.as_str(), *c)).collect();
        let mut matches: HashMap<String, Vec<Value>> = HashMap::new();
        for person in &data.snapshot.contributors {
            if person.kind != "HUMAN" {
                continue;
            }
            let mut seen = HashSet::new();
            for affiliation in
                profile_affiliations(&person.affiliation, &person.affiliations, &company_map, &date_str(end))
            {
                let bucket = affiliation["company_id"].as_str().unwrap_or_default().to_string();
                if !seen.insert(bucket.clone()) {
                    continue;
                }
                let name = [&person.display_name, &person.login, &person.id]
                    .into_iter()
                    .find(|v| !v.is_empty())
                    .cloned()
                    .unwrap_or_default();
                matches.entry(bucket).or_default().push(json!({
                    "id": person.id,
                    "login": person.login,
                    "name": name,
                    "affiliation": affiliation,
                }));
            }
        }

        let metric_values = |values: &[i64; 5]| -> serde_json::Map<String, Value> {
            CompanyMetricKind::ALL.iter().map(|k| (k.name().to_string(), json!(values[*k as usize]))).collect()
        };
        let mut rows: Vec<Value> = companies
            .iter()
            .enumerate()
            .map(|(index, company)| {
                let mut row = serde_json::to_value(company).unwrap_or_default();
                if company.id == "independent" {
                    row.as_object_mut().map(|o| o.remove("aliases"));
                }
                let values = totals.get(&(index as u32)).copied().unwrap_or_default();
                for (key, value) in metric_values(&values) {
                    row[key] = value;
                }
                row["profile_matches"] = json!(matches.get(&company.id).map_or(0, Vec::len));
                row
            })
            .collect();
        rows.sort_by(|a, b| {
            let key = |v: &Value| {
                (
                    -v["pull_requests_opened"].as_i64().unwrap_or(0),
                    -v["commits"].as_i64().unwrap_or(0),
                    -v["profile_matches"].as_i64().unwrap_or(0),
                    fold(v["canonical_name"].as_str().unwrap_or_default()),
                )
            };
            key(a).cmp(&key(b))
        });
        let percent = if coverage_total > 0 {
            (1000.0 * coverage_reviewed as f64 / coverage_total as f64).round() / 10.0
        } else {
            0.0
        };
        let mut result = json!({
            "companies": rows,
            "filters": {"days": days_value(days), "repository_id": repository_id, "target_branch": target_branch},
            "repository_options": repository_options,
            "coverage": {"events": coverage_total, "reviewed_events": coverage_reviewed, "percent": percent},
            "mapping_revision": data.snapshot.meta.mapping_revision,
            "observed_at": iso(Some(data.snapshot.meta.generated_at)),
        });
        if let Some(index) = selected {
            let company = companies[index as usize];
            let values = totals.get(&index).copied().unwrap_or_default();
            let mut profile_matches = matches.remove(&company.id).unwrap_or_default();
            profile_matches.sort_by_key(|row| {
                (
                    !row["affiliation"]["is_current"].as_bool().unwrap_or(false),
                    fold(row["name"].as_str().unwrap_or_default()),
                )
            });
            let mut company_value = serde_json::to_value(company).unwrap_or_default();
            if company.id == "independent" {
                company_value.as_object_mut().map(|o| o.remove("aliases"));
            }
            result["company"] = company_value;
            result["summary"] = Value::Object(metric_values(&values));
            result["profile_matches"] = json!(profile_matches);
            result["daily"] = json!(
                daily
                    .iter()
                    .map(|(day, v)| json!({
                        "date": date_str(*day),
                        "commits": v[CompanyMetricKind::Commits as usize],
                        "pull_requests_merged": v[CompanyMetricKind::PullRequestsMerged as usize],
                    }))
                    .collect::<Vec<_>>()
            );
            result["repository_activity"] = json!(repository_totals
                .values()
                .map(|(repo, branch, v)| {
                    let mut row = json!({
                        "id": data.repo(*repo).id,
                        "target_branch": data.branch(*branch),
                        "name": repo_names.get(&(*repo, *branch)).cloned().unwrap_or_else(|| data.repo(*repo).id.clone()),
                    });
                    for (key, value) in metric_values(v) {
                        row[key] = value;
                    }
                    row
                })
                .collect::<Vec<_>>());
        }
        Ok(result)
    }
}
