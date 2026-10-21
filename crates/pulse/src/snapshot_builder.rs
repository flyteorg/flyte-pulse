//! Build the browser snapshot from the collected state.

use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use flate2::Compression;
use flate2::write::GzEncoder;
use pulse_core::snapshot::{
    AiIdentity, AiName, CommitAi, CommitRecord, Company, CompanyMetric, Coverage, Meta, Metric, NONE, Person, Pr, Repo,
    Snapshot, Trailer, parse_sha,
};
use pulse_core::state::{Commit, State};
use pulse_core::time::{format_date, format_timestamp};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::attribution::{
    TrailerRole, ai_model_identity, commit_subject, is_ai_name, normalized_ai_name, resolve_trailer_contributor,
    trailer_person, trailers,
};
use crate::companies::CompanyCatalog;
use crate::config::hex;
use crate::metrics::{Facts, build_company_metrics, build_daily_metrics};

struct Interner<'a> {
    facts: &'a Facts<'a>,
    contributor_index: HashMap<String, u32>,
    extra_keys: Vec<String>,
    extra_index: HashMap<String, u32>,
    branches: Vec<String>,
    branch_index: HashMap<String, u32>,
    identities: Vec<AiIdentity>,
    identity_index: HashMap<AiIdentity, u32>,
}

impl<'a> Interner<'a> {
    fn new(facts: &'a Facts<'a>) -> Self {
        let contributor_index =
            facts.state.contributors.iter().enumerate().map(|(i, c)| (c.id.clone(), i as u32)).collect();
        Self {
            facts,
            contributor_index,
            extra_keys: Vec::new(),
            extra_index: HashMap::new(),
            branches: vec![String::new()],
            branch_index: HashMap::from([(String::new(), 0)]),
            identities: Vec::new(),
            identity_index: HashMap::new(),
        }
    }

    fn key(&mut self, id: &str) -> u32 {
        if id.is_empty() {
            return NONE;
        }
        if let Some(&index) = self.contributor_index.get(id) {
            return index;
        }
        let base = self.facts.state.contributors.len() as u32;
        if let Some(&index) = self.extra_index.get(id) {
            return base + index;
        }
        let index = self.extra_keys.len() as u32;
        self.extra_keys.push(id.to_string());
        self.extra_index.insert(id.to_string(), index);
        base + index
    }

    fn branch(&mut self, name: &str) -> u32 {
        if let Some(&index) = self.branch_index.get(name) {
            return index;
        }
        let index = self.branches.len() as u32;
        self.branches.push(name.to_string());
        self.branch_index.insert(name.to_string(), index);
        index
    }

    fn identity(&mut self, name: &str) -> u32 {
        let identity = ai_model_identity(name);
        let identity =
            AiIdentity { key: identity.key, provider: identity.provider, model: identity.model, name: identity.name };
        if let Some(&index) = self.identity_index.get(&identity) {
            return index;
        }
        let index = self.identities.len() as u32;
        self.identities.push(identity.clone());
        self.identity_index.insert(identity, index);
        index
    }

    /// The AI identity of a contributor or name that is AI by identity kind or by name.
    fn provider_identity(&mut self, value: &str, contributor_id: &str) -> u32 {
        let facts = self.facts;
        let is_ai = facts.kind(contributor_id) == "AI" || is_ai_name(value, &facts.known_ai_names);
        if !is_ai {
            return NONE;
        }
        let name = facts.label(contributor_id).unwrap_or(value).to_string();
        self.identity(&name)
    }

    /// The AI contributor whose display name or login matches a trailer name.
    fn ai_contributor(&mut self, name: &str) -> u32 {
        let normalized = normalized_ai_name(name);
        if normalized.is_empty() {
            return NONE;
        }
        let found = self.facts.state.contributors.iter().find(|c| {
            c.identity_kind.eq_ignore_ascii_case("AI")
                && (normalized_ai_name(&c.display_name) == normalized || normalized_ai_name(&c.login) == normalized)
        });
        found.map(|c| self.key(&c.id)).unwrap_or(NONE)
    }
}

fn commit_ai(interner: &mut Interner, commit: &Commit) -> Option<Box<CommitAi>> {
    let facts = interner.facts;
    let author_id = commit.author_contributor_id.as_str();
    let ai_author = facts.kind(author_id) == "AI";
    let author_label = facts.label(author_id).unwrap_or(&commit.author_name).to_string();
    let author_provider_identity = interner.provider_identity(&author_label, author_id);
    let parsed = trailers(&commit.message);
    if !ai_author && author_provider_identity == NONE && parsed.is_empty() {
        return None;
    }
    let ai_author_identity = if ai_author {
        let name = if author_label.is_empty() { "AI author".to_string() } else { author_label.clone() };
        interner.identity(&name)
    } else {
        NONE
    };
    let mut coauthor_names = BTreeSet::new();
    let mut assistant_names = BTreeSet::new();
    let mut has_coauthor_trailer = false;
    let mut has_assisted_trailer = false;
    let mut trailer_records = Vec::new();
    for (role, raw) in &parsed {
        let person = trailer_person(raw);
        let is_ai = is_ai_name(&person, &facts.known_ai_names);
        match role {
            TrailerRole::CoAuthoredBy => {
                has_coauthor_trailer = true;
                if is_ai {
                    coauthor_names.insert(person.clone());
                }
            }
            TrailerRole::AssistedBy => {
                has_assisted_trailer = true;
                if is_ai {
                    assistant_names.insert(person.clone());
                }
            }
        }
        let resolved = resolve_trailer_contributor(raw, &facts.aliases);
        let mut ai_identity = interner.provider_identity(&person, &resolved);
        if ai_identity == NONE && is_ai {
            ai_identity = interner.identity(&person);
        }
        trailer_records.push(Trailer {
            co_authored: *role == TrailerRole::CoAuthoredBy,
            resolved: interner.key(&resolved),
            ai_identity,
        });
    }
    let mut names = |set: BTreeSet<String>| -> Vec<AiName> {
        set.into_iter()
            .map(|name| AiName {
                contributor: interner.ai_contributor(&name),
                identity: interner.identity(&name),
                name,
            })
            .collect()
    };
    let coauthors = names(coauthor_names);
    let assistants = names(assistant_names);
    Some(Box::new(CommitAi {
        ai_author,
        ai_author_identity,
        author_provider_identity,
        coauthors,
        assistants,
        has_coauthor_trailer,
        has_assisted_trailer,
        trailers: trailer_records,
    }))
}

pub fn build_snapshot(state: &State, companies: &CompanyCatalog, generated_at: i64) -> Snapshot {
    let facts = Facts::new(state);
    let mut interner = Interner::new(&facts);
    let today = format_date(pulse_core::time::day_of(generated_at));

    let repositories: Vec<Repo> = state
        .repositories
        .iter()
        .map(|r| Repo {
            id: r.repository_id.clone(),
            owner: r.owner.clone(),
            name: r.name.clone(),
            module: r.module.clone(),
            status: r.status.clone(),
            target_branches: r.target_branches.clone(),
            languages: r.languages.clone(),
        })
        .collect();
    let repo_index: HashMap<&str, u32> =
        repositories.iter().enumerate().map(|(i, r)| (r.id.as_str(), i as u32)).collect();
    for repository in &repositories {
        for branch in &repository.target_branches {
            interner.branch(&branch.name);
        }
    }

    let contributors: Vec<Person> = state
        .contributors
        .iter()
        .map(|c| Person {
            id: c.id.clone(),
            login: c.login.clone(),
            display_name: c.display_name.clone(),
            profile_url: c.profile_url.clone(),
            kind: if c.identity_kind.is_empty() { "UNKNOWN".into() } else { c.identity_kind.clone() },
            kind_source: if c.identity_kind_source.is_empty() {
                "UNKNOWN".into()
            } else {
                c.identity_kind_source.clone()
            },
            profile_company: c.profile_company.clone(),
            is_bot: c.is_bot,
            affiliation: companies.current_affiliation(c, &today),
            affiliations: companies.periods.get(&c.id).cloned().unwrap_or_default(),
            maintainer_days: companies.maintainer_days(&c.id),
        })
        .collect();

    let mut metrics = Vec::new();
    for ((date, repo, key, kind, branch), value) in build_daily_metrics(&facts) {
        let Some(&repo) = repo_index.get(repo.as_str()) else { continue };
        metrics.push(Metric { date, repo, key: interner.key(&key), kind, branch: interner.branch(&branch), value });
    }

    let company_index: HashMap<&str, u32> =
        companies.companies.iter().enumerate().map(|(i, c)| (c.id.as_str(), i as u32)).collect();
    let independent = companies.companies.len() as u32;
    let company_data = build_company_metrics(&facts, companies);
    let mut company_metrics = Vec::new();
    for ((date, repo, branch, company, kind), value) in company_data.values {
        let Some(&repo) = repo_index.get(repo.as_str()) else { continue };
        let company = company_index.get(company.as_str()).copied().unwrap_or(independent);
        company_metrics.push(CompanyMetric { date, repo, branch: interner.branch(&branch), company, kind, value });
    }
    let mut company_coverage = Vec::new();
    for ((date, repo, branch, kind), (value, reviewed)) in company_data.coverage {
        let Some(&repo) = repo_index.get(repo.as_str()) else { continue };
        company_coverage.push(Coverage { date, repo, branch: interner.branch(&branch), kind, value, reviewed });
    }

    let mut pull_requests = Vec::new();
    let mut pr_index = HashMap::new();
    for pr in &state.pull_requests {
        let Some(&repo) = repo_index.get(pr.repository_id.as_str()) else { continue };
        let author_id = pr.author_contributor_id.as_str();
        let author_ai_identity = if facts.kind(author_id) == "AI" {
            let name = facts.label(author_id).unwrap_or(author_id).to_string();
            interner.identity(&name)
        } else {
            NONE
        };
        let label = facts.label(author_id).unwrap_or_default().to_string();
        let author_provider_identity = interner.provider_identity(&label, author_id);
        pr_index.insert((pr.repository_id.as_str(), pr.provider_id.as_str()), pull_requests.len() as u32);
        pull_requests.push(Pr {
            repo,
            provider_id: pr.provider_id.clone(),
            number: pr.number,
            author: interner.key(author_id),
            title: pr.title.clone(),
            state: pr.state.clone(),
            created_at: pr.created_at.map(|t| t.0),
            merged_at: pr.merged_at.map(|t| t.0),
            first_review_at: facts.reviewers(pr).into_values().min().map(|t| t.0),
            branch: interner.branch(&pr.target_branch),
            additions: pr.additions,
            deletions: pr.deletions,
            commit_count: pr.commit_count,
            author_ai_identity,
            author_provider_identity,
        });
    }

    let mut commits = Vec::new();
    let mut commit_index = HashMap::new();
    for commit in facts.counted_commits() {
        let (Some(&repo), Some(sha)) = (repo_index.get(commit.repository_id.as_str()), parse_sha(&commit.sha)) else {
            continue;
        };
        let has_label = |id: &str| facts.label(id).is_some();
        commit_index.insert((commit.repository_id.as_str(), commit.sha.as_str()), commits.len() as u32);
        let ai = commit_ai(&mut interner, commit);
        commits.push(CommitRecord {
            repo,
            sha,
            author: interner.key(&commit.author_contributor_id),
            committer: interner.key(&commit.committer_contributor_id),
            author_name: if has_label(&commit.author_contributor_id) {
                String::new()
            } else {
                commit.author_name.clone()
            },
            committer_name: if has_label(&commit.committer_contributor_id) {
                String::new()
            } else {
                commit.committer_name.clone()
            },
            committed_at: commit.committed_at.map(|t| t.0),
            subject: commit_subject(&commit.message),
            ai,
        });
    }

    let mut pr_commits: Vec<(u32, u32)> = state
        .pull_request_commits
        .iter()
        .filter_map(|link| {
            let pr = pr_index.get(&(link.repository_id.as_str(), link.pull_request_id.as_str()))?;
            let commit = commit_index.get(&(link.repository_id.as_str(), link.commit_sha.as_str()))?;
            Some((*pr, *commit))
        })
        .collect();
    pr_commits.sort_unstable();
    pr_commits.dedup();

    let raw_generated_at = state.generated_at.map(|t| t.0).unwrap_or(generated_at);
    let compact = |ts: i64| format_timestamp(ts).replace(['-', ':'], "");
    Snapshot {
        meta: Meta {
            version: compact(generated_at),
            generated_at,
            raw_generated_at,
            raw_version: compact(raw_generated_at),
            catalog_revision: state.catalog_revision.clone(),
            mapping_revision: companies.revision.clone(),
        },
        repositories,
        contributors,
        extra_keys: interner.extra_keys,
        branches: interner.branches,
        companies: companies
            .companies
            .iter()
            .map(|c| Company {
                id: c.id.clone(),
                canonical_name: c.canonical_name.clone(),
                aliases: c.aliases.clone(),
                website_url: c.website_url.clone(),
                maintainer: c.maintainer,
            })
            .collect(),
        metrics,
        company_metrics,
        company_coverage,
        ai_identities: interner.identities,
        pull_requests,
        commits,
        pr_commits,
    }
}

/// Write `pulse.bin.gz` and `meta.json` into `out_dir`.
pub fn write_snapshot(snapshot: &Snapshot, out_dir: &Path) -> Result<serde_json::Value> {
    std::fs::create_dir_all(out_dir)?;
    let encoded = snapshot.encode()?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(&encoded)?;
    let compressed = encoder.finish()?;
    let file = "pulse.bin.gz";
    std::fs::write(out_dir.join(file), &compressed).with_context(|| format!("cannot write {file}"))?;
    let meta = json!({
        "version": snapshot.meta.version,
        "generated_at": format_timestamp(snapshot.meta.generated_at),
        "raw_generated_at": format_timestamp(snapshot.meta.raw_generated_at),
        "file": file,
        "size": compressed.len(),
        "uncompressed_size": encoded.len(),
        "sha256": hex(&Sha256::digest(&compressed)),
        "counts": {
            "repositories": snapshot.repositories.len(),
            "contributors": snapshot.contributors.len(),
            "pull_requests": snapshot.pull_requests.len(),
            "commits": snapshot.commits.len(),
            "metrics": snapshot.metrics.len(),
            "company_metrics": snapshot.company_metrics.len(),
        },
    });
    std::fs::write(out_dir.join("meta.json"), serde_json::to_vec_pretty(&meta)?)?;
    Ok(meta)
}
