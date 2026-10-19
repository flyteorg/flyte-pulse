//! Company and affiliation discovery. Finds candidates for review from public
//! profile companies, public organization memberships, and corporate commit email
//! domains, including when an affiliation changed. Nothing is applied without a
//! reviewed pull request.

mod edit;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pulse_core::state::{Contributor, State};
use pulse_core::time::{SECONDS_PER_DAY, day_of, format_date};
use serde::Deserialize;

pub use edit::apply;

use crate::companies::{CompanyCatalog, load_company_catalog, normalize_company};
use crate::config::{apply_identity_rules, bot_name, load_catalog};
use crate::report::{append, thousands, today};
use crate::state_io::read_state;
use crate::sync::github::Organization;
use crate::sync::github_client;

#[derive(clap::Args, Debug)]
pub struct DiscoverArgs {
    /// Collected state: a .json.gz/.json file or a JSON-lines directory.
    #[arg(long, default_value = "pulse-state.json.gz")]
    pub state: PathBuf,
    /// Directory with repositories.yaml, companies.yaml, affiliations.yaml, and discovery.yaml.
    #[arg(long, default_value = "config")]
    pub config_dir: PathBuf,
    /// Edit companies.yaml and affiliations.yaml with the candidates.
    #[arg(long)]
    pub apply: bool,
    /// Write the Markdown report to PATH, for example as a pull request description.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,
    /// Append the report to PATH, such as `$GITHUB_STEP_SUMMARY`.
    #[arg(long, value_name = "PATH")]
    pub summary_file: Option<PathBuf>,
    /// Skip GitHub organization lookups (no new companies are proposed).
    #[arg(long)]
    pub offline: bool,
}

pub fn run(args: DiscoverArgs) -> Result<()> {
    let mut state = read_state(&args.state)?;
    let repositories = load_catalog(&args.config_dir.join("repositories.yaml"))?;
    apply_identity_rules(&mut state.contributors, &repositories.identity_rules);
    let catalog = load_company_catalog(&args.config_dir, &state.contributors)?;
    let ignore = Ignore::load(&args.config_dir)?;
    let candidates = organization_candidates(&state, &catalog, &ignore);
    let organizations = if args.offline || candidates.is_empty() {
        HashMap::new()
    } else {
        match github_client().and_then(|mut github| github.organizations(&candidates)) {
            Ok(found) => found,
            Err(error) => {
                println!("::warning::Skipping GitHub organization lookups: {error:#}");
                HashMap::new()
            }
        }
    };
    let today = today();
    let discovery = discover(&state, &catalog, &ignore, today, &organizations);
    let mut note = None;
    if args.apply && discovery.has_edits() {
        note = Some(match apply(&discovery, &args.config_dir, &state.contributors, &format_date(today)) {
            Ok(()) => "This pull request applies the candidates below to `config/companies.yaml` and \
                       `config/affiliations.yaml`. Merging it approves them; edit or remove entries that are wrong."
                .to_string(),
            Err(error) => {
                // Report instead of failing: discovery must never block the sync and deploy.
                println!("::warning::Company discovery edits were not applied: {error:#}");
                format!("> [!WARNING]\n> The candidates below were not applied: {error:#}")
            }
        });
    }
    let applied = note.as_deref();
    let report = markdown(&discovery, &catalog, applied);
    if let Some(path) = &args.report {
        std::fs::write(path, &report)?;
    }
    if let Some(path) = &args.summary_file {
        append(path, &report)?;
    }
    if args.report.is_none() && args.summary_file.is_none() {
        print!("{report}");
    }
    eprintln!(
        "discovery: {} companies, {} domains, {} affiliations, {} changes, {} earlier periods, {} for review{}",
        discovery.companies.len(),
        discovery.domains.len(),
        discovery.affiliations.len(),
        discovery.changes.len(),
        discovery.periods.len(),
        discovery.review.len(),
        if applied.is_some_and(|n| !n.contains("not applied")) { " (applied)" } else { "" }
    );
    Ok(())
}

/// How recent a commit email domain must be to describe the current affiliation.
const CURRENT_DOMAIN_DAYS: i32 = 365;
/// Overlap tolerated between consecutive email domains when dating a switch.
const DOMAIN_OVERLAP_DAYS: i64 = 30;
/// A longer gap before the next domain ends a period the day after its last commit.
const DOMAIN_GAP_DAYS: i64 = 90;

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DiscoveryFile {
    ignore: IgnoreFile,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct IgnoreFile {
    profile_companies: Vec<String>,
    organizations: Vec<String>,
    domains: Vec<String>,
    users: Vec<String>,
}

/// Values that are never proposed again (`config/discovery.yaml`).
#[derive(Default)]
pub struct Ignore {
    profile_companies: HashSet<String>,
    organizations: HashSet<String>,
    domains: HashSet<String>,
    users: HashSet<String>,
}

impl Ignore {
    pub fn load(config_dir: &Path) -> Result<Self> {
        let path = config_dir.join("discovery.yaml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let file: DiscoveryFile =
            serde_yaml_ng::from_slice(&std::fs::read(&path)?).with_context(|| format!("invalid {}", path.display()))?;
        let lower = |values: Vec<String>| -> HashSet<String> {
            values.into_iter().map(|v| v.trim().trim_start_matches('@').to_lowercase()).collect()
        };
        Ok(Self {
            profile_companies: file.ignore.profile_companies.iter().map(|v| normalize_company(v)).collect(),
            organizations: lower(file.ignore.organizations),
            domains: lower(file.ignore.domains),
            users: lower(file.ignore.users),
        })
    }
}

#[derive(Clone, Debug)]
pub struct DomainMapping {
    pub company_id: String,
    pub domain: String,
    pub evidence: String,
}

/// A GitHub organization handle for an existing company, from its organization profile.
#[derive(Clone, Debug)]
pub struct AliasMapping {
    pub company_id: String,
    pub alias: String,
    pub logins: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct NewCompany {
    pub id: String,
    pub canonical_name: String,
    pub website_url: String,
    pub alias: String,
    pub domains: Vec<String>,
    pub logins: Vec<String>,
    pub evidence: String,
}

#[derive(Clone, Debug)]
pub struct NewAffiliation {
    pub login: String,
    pub company_id: String,
    pub evidence: String,
}

#[derive(Clone, Debug)]
pub struct Change {
    pub login: String,
    pub contributor_id: String,
    /// The currently effective company ("" for Independent).
    pub from: String,
    /// The new company ("" when the contributor left without a known new company).
    pub to: String,
    /// First day (UTC) of the new affiliation.
    pub date: String,
    /// The contributor's first recorded activity, the default start of the old period.
    pub first_activity: String,
    pub evidence: String,
}

#[derive(Clone, Debug)]
pub struct Period {
    pub login: String,
    pub company_id: String,
    pub valid_from: String,
    pub valid_until: String,
    pub evidence: String,
}

#[derive(Clone, Debug)]
pub struct ReviewItem {
    pub kind: &'static str,
    pub subject: String,
    pub detail: String,
}

#[derive(Default, Debug)]
pub struct Discovery {
    pub aliases: Vec<AliasMapping>,
    pub domains: Vec<DomainMapping>,
    pub companies: Vec<NewCompany>,
    pub affiliations: Vec<NewAffiliation>,
    pub changes: Vec<Change>,
    pub periods: Vec<Period>,
    pub review: Vec<ReviewItem>,
}

impl Discovery {
    /// Whether there is anything to edit in the config files.
    pub fn has_edits(&self) -> bool {
        !(self.aliases.is_empty()
            && self.domains.is_empty()
            && self.companies.is_empty()
            && self.affiliations.is_empty()
            && self.changes.is_empty()
            && self.periods.is_empty())
    }
}

/// Commits from one email domain: first and last authored time and count.
#[derive(Clone, Debug)]
struct DomainRun {
    domain: String,
    first: i64,
    last: i64,
    commits: usize,
}

fn domain_runs(state: &State) -> HashMap<&str, Vec<DomainRun>> {
    let mut runs: HashMap<&str, BTreeMap<&str, DomainRun>> = HashMap::new();
    for commit in &state.commits {
        let at = commit.authored_at.or(commit.committed_at);
        if commit.author_email_domain.is_empty() || commit.author_contributor_id.is_empty() {
            continue;
        }
        let Some(at) = at else { continue };
        let run =
            runs.entry(&commit.author_contributor_id).or_default().entry(&commit.author_email_domain).or_insert(
                DomainRun { domain: commit.author_email_domain.clone(), first: at.0, last: at.0, commits: 0 },
            );
        run.first = run.first.min(at.0);
        run.last = run.last.max(at.0);
        run.commits += 1;
    }
    runs.into_iter()
        .map(|(id, by_domain)| {
            let mut list: Vec<DomainRun> = by_domain.into_values().collect();
            list.sort_by_key(|r| (r.first, r.last));
            (id, list)
        })
        .collect()
}

fn date(timestamp: i64) -> String {
    format_date(day_of(timestamp))
}

/// `@org` handles in a profile company string such as `@lyft @stripe`.
fn handles(profile: &str) -> Vec<String> {
    profile
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '|' | '/' | ';' | '(' | ')'))
        .filter_map(|token| token.strip_prefix('@'))
        .map(|login| login.trim_end_matches(|c: char| !c.is_ascii_alphanumeric()))
        .filter(|login| !login.is_empty() && login.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .map(str::to_string)
        .collect()
}

/// A company ID for a new company: lowercase letters, digits, and dashes.
fn company_id(login: &str, taken: &HashSet<String>) -> String {
    let mut id: String =
        login.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    id = id.trim_matches('-').to_string();
    if !id.starts_with(|c: char| c.is_ascii_lowercase()) {
        id = format!("org-{id}");
    }
    if matches!(id.as_str(), "unknown" | "independent" | "unmapped") {
        id = format!("{id}-org");
    }
    let mut candidate = id.clone();
    let mut suffix = 2;
    while taken.contains(&candidate) {
        candidate = format!("{id}-{suffix}");
        suffix += 1;
    }
    candidate
}

/// An HTTPS website URL from a GitHub organization profile, if usable.
fn https_website(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    (host.contains('.') && !host.contains('@')).then(|| format!("https://{rest}"))
}

fn is_human(contributor: &Contributor) -> bool {
    contributor.identity_kind.eq_ignore_ascii_case("HUMAN")
}

fn profile_link(contributor: &Contributor) -> String {
    format!("[@{0}](https://github.com/{0})", contributor.login)
}

/// Unknown accounts named like bots are reported, never reclassified or affiliated.
fn suspected_bot(contributor: &Contributor, catalog: &CompanyCatalog) -> bool {
    is_human(contributor)
        && contributor.identity_kind_source != "CATALOG_RULE"
        && !catalog.current.contains_key(&contributor.id)
        && !catalog.periods.contains_key(&contributor.id)
        && bot_name(&contributor.login, &contributor.display_name)
}

/// Human contributors considered for affiliations, sorted by login.
fn people<'a>(state: &'a State, catalog: &CompanyCatalog, ignore: &Ignore) -> Vec<&'a Contributor> {
    let mut list: Vec<&Contributor> = state
        .contributors
        .iter()
        .filter(|c| is_human(c) && !suspected_bot(c, catalog) && !ignore.users.contains(&c.login.to_lowercase()))
        .collect();
    list.sort_by_key(|c| c.login.to_lowercase());
    list
}

type DomainUsage<'a> = BTreeMap<&'a str, Vec<(&'a str, &'a DomainRun)>>;

/// Commit email domains not mapped to a company, with the people using them.
fn domain_usage<'a>(
    people: &[&'a Contributor],
    runs: &'a HashMap<&str, Vec<DomainRun>>,
    catalog: &CompanyCatalog,
    ignore: &Ignore,
) -> DomainUsage<'a> {
    let mut usage: DomainUsage = BTreeMap::new();
    for contributor in people {
        for run in runs.get(contributor.id.as_str()).into_iter().flatten() {
            if !ignore.domains.contains(&run.domain) && catalog.domain_company(&run.domain).is_none() {
                usage.entry(&run.domain).or_default().push((&contributor.id, run));
            }
        }
    }
    usage
}

/// The company whose website host is this domain, a parent of it, or a subdomain of it
/// (`opensource.expediagroup.com` for `expediagroup.com`).
fn website_company<'a>(catalog: &'a CompanyCatalog, domain: &str) -> Option<&'a str> {
    catalog.companies.iter().map(|c| c.id.as_str()).find(|id| {
        let host = catalog.website_host(id);
        !host.is_empty()
            && (domain == host || domain.ends_with(&format!(".{host}")) || host.ends_with(&format!(".{domain}")))
    })
}

/// The GitHub organization a company domain would use: `dominodatalab.com` → `dominodatalab`.
fn domain_stem(domain: &str) -> Option<String> {
    let labels: Vec<&str> = domain.split('.').collect();
    let n = labels.len();
    if n < 2 {
        return None;
    }
    // Two-part public suffixes such as co.uk or com.au.
    let second_level = ["ac", "co", "com", "edu", "gov", "net", "org"];
    let index = if n >= 3 && second_level.contains(&labels[n - 2]) && labels[n - 1].len() == 2 { n - 3 } else { n - 2 };
    Some(labels[index].to_string())
}

fn url_host(url: &str) -> String {
    let rest = url.trim().trim_start_matches("https://").trim_start_matches("http://");
    rest.split(['/', '?', '#']).next().unwrap_or_default().trim_start_matches("www.").to_lowercase()
}

/// The single organization handle in a profile company field, ignoring listed
/// organizations; `None` for multiple handles or none.
fn single_handle(profile: &str, ignore: &Ignore) -> Option<String> {
    let found: Vec<String> =
        handles(profile).into_iter().filter(|h| !ignore.organizations.contains(&h.to_lowercase())).collect();
    match found.as_slice() {
        [handle] => Some(handle.clone()),
        _ => None,
    }
}

/// GitHub organizations to look up: handles named alone in profile company fields
/// that match no company, and names matching unmapped email domains used by
/// several people.
pub fn organization_candidates(state: &State, catalog: &CompanyCatalog, ignore: &Ignore) -> Vec<String> {
    let people = people(state, catalog, ignore);
    let mut logins = BTreeSet::new();
    for contributor in &people {
        let profile = contributor.profile_company.trim();
        if profile.is_empty() || catalog.aliases.contains_key(&normalize_company(profile)) {
            continue;
        }
        if let Some(handle) = single_handle(profile, ignore)
            && catalog.organization_company(&handle).is_none()
        {
            logins.insert(handle);
        }
    }
    let runs = domain_runs(state);
    for (domain, users) in domain_usage(&people, &runs, catalog, ignore) {
        if users.len() >= 2 && website_company(catalog, domain).is_none() {
            logins.extend(domain_stem(domain));
        }
    }
    logins.into_iter().collect()
}

/// Find candidates. `organizations` holds lookups for [`organization_candidates`].
pub fn discover(
    state: &State,
    catalog: &CompanyCatalog,
    ignore: &Ignore,
    today: i32,
    organizations: &HashMap<String, Option<Organization>>,
) -> Discovery {
    let mut discovery = Discovery::default();
    let mut bots: Vec<&Contributor> = state.contributors.iter().collect();
    bots.sort_by_key(|c| c.login.to_lowercase());
    for contributor in bots {
        if matches!(contributor.identity_kind_source.as_str(), "BOT_NAME_HEURISTIC" | "BOT_EMAIL_HEURISTIC") {
            discovery.review.push(ReviewItem {
                kind: "Classified as bot",
                subject: profile_link(contributor),
                detail: "a new account whose login, name, or commit email contains \"bot\"; add an identity rule \
                         with `kind: HUMAN` if this is a person"
                    .into(),
            });
        } else if suspected_bot(contributor, catalog) {
            discovery.review.push(ReviewItem {
                kind: "Possible bot",
                subject: profile_link(contributor),
                detail: format!(
                    "\"{}\" contains \"bot\"; add an identity rule with `kind: BOT` if this is a bot",
                    contributor.display_name
                ),
            });
        }
    }
    let humans = people(state, catalog, ignore);
    let runs = domain_runs(state);
    let mut taken: HashSet<String> = catalog.by_id.keys().cloned().collect();

    // 1. Email domains used by contributors but not mapped to a company.
    let mut domain_proposals: HashMap<String, String> = HashMap::new();
    let usage = domain_usage(&humans, &runs, catalog, ignore);
    for (domain, users) in &usage {
        let commits: usize = users.iter().map(|(_, run)| run.commits).sum();
        let summary = format!("{} commits from {} contributors", thousands(commits as u64), users.len());
        if let Some(company) = website_company(catalog, domain) {
            domain_proposals.insert(domain.to_string(), company.to_string());
            discovery.domains.push(DomainMapping {
                company_id: company.to_string(),
                domain: domain.to_string(),
                evidence: format!("matches the website {}; {summary}", catalog.website_host(company)),
            });
            continue;
        }
        let mut votes: BTreeMap<String, usize> = BTreeMap::new();
        let mut curated = 0;
        for (id, run) in users {
            let company = catalog.historical(id, &date(run.last));
            if !company.is_empty() {
                *votes.entry(company).or_default() += 1;
                curated += 1;
            }
        }
        if let Some((company, &n)) = votes.iter().max_by_key(|(_, n)| **n)
            && n >= 2
            && n * 5 >= curated * 4
        {
            domain_proposals.insert(domain.to_string(), company.clone());
            discovery.domains.push(DomainMapping {
                company_id: company.clone(),
                domain: domain.to_string(),
                evidence: format!("{n} of {curated} curated contributors committing from it are affiliated; {summary}"),
            });
            continue;
        }
        if users.len() < 2 {
            continue;
        }
        // A GitHub organization whose website is this domain identifies the company.
        let organization =
            domain_stem(domain).and_then(|stem| organizations.get(&stem).cloned().flatten()).filter(|org| {
                let host = url_host(&org.website_url);
                !host.is_empty() && (*domain == host || domain.ends_with(&format!(".{host}")))
            });
        match organization {
            Some(org) => {
                let evidence = format!(
                    "GitHub organization @{} with website {}; {summary}",
                    org.login,
                    url_host(&org.website_url)
                );
                let existing = [&org.name, &org.login]
                    .into_iter()
                    .find_map(|value| catalog.aliases.get(&normalize_company(value)).cloned());
                if let Some(company) = existing {
                    domain_proposals.insert(domain.to_string(), company.clone());
                    discovery.domains.push(DomainMapping { company_id: company, domain: domain.to_string(), evidence });
                } else if let Some(website_url) = https_website(&org.website_url) {
                    let id = company_id(&org.login, &taken);
                    taken.insert(id.clone());
                    domain_proposals.insert(domain.to_string(), id.clone());
                    discovery.companies.push(NewCompany {
                        id,
                        canonical_name: if org.name.trim().is_empty() {
                            org.login.clone()
                        } else {
                            org.name.trim().into()
                        },
                        website_url,
                        alias: format!("@{}", org.login),
                        domains: vec![domain.to_string()],
                        logins: Vec::new(),
                        evidence,
                    });
                }
            }
            None => discovery.review.push(ReviewItem {
                kind: "Unmapped email domain",
                subject: domain.to_string(),
                detail: summary,
            }),
        }
    }
    let domain_company = |domain: &str| -> Option<String> {
        catalog.domain_company(domain).map(str::to_string).or_else(|| domain_proposals.get(domain).cloned())
    };

    // 2. New companies from organizations named alone in profile company fields.
    let mut new_orgs: HashMap<String, String> = HashMap::new();
    let mut profile_orgs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for contributor in &humans {
        let profile = contributor.profile_company.trim();
        if catalog.aliases.contains_key(&normalize_company(profile)) {
            continue;
        }
        if let Some(handle) = single_handle(profile, ignore)
            && catalog.organization_company(&handle).is_none()
        {
            profile_orgs.entry(handle.to_lowercase()).or_default().push(contributor.login.clone());
        }
    }
    for (login, users) in profile_orgs {
        let organization = organizations
            .get(&login)
            .or_else(|| organizations.iter().find(|(key, _)| key.eq_ignore_ascii_case(&login)).map(|(_, value)| value));
        let Some(organization) = organization else { continue };
        // An organization of a known company: add its handle as an alias.
        let existing = organization.as_ref().and_then(|org| {
            [&org.name, &org.login]
                .into_iter()
                .find_map(|value| catalog.aliases.get(&normalize_company(value)))
                .map(|id| (org, id.clone()))
        });
        if let Some((org, id)) = existing {
            new_orgs.insert(login.clone(), id.clone());
            discovery.aliases.push(AliasMapping { company_id: id, alias: format!("@{}", org.login), logins: users });
            continue;
        }
        let proposed = organization.as_ref().and_then(|org| {
            discovery
                .companies
                .iter()
                .find(|c| c.alias.eq_ignore_ascii_case(&format!("@{}", org.login)))
                .map(|c| c.id.clone())
        });
        if let Some(id) = proposed {
            new_orgs.insert(login.clone(), id.clone());
            if let Some(company) = discovery.companies.iter_mut().find(|c| c.id == id) {
                company.logins.extend(users);
            }
            continue;
        }
        match organization.as_ref().and_then(|o| https_website(&o.website_url).map(|url| (o, url))) {
            Some((org, website_url)) => {
                let id = company_id(&org.login, &taken);
                taken.insert(id.clone());
                new_orgs.insert(login.clone(), id.clone());
                discovery.companies.push(NewCompany {
                    id,
                    canonical_name: if org.name.trim().is_empty() { org.login.clone() } else { org.name.trim().into() },
                    website_url,
                    alias: format!("@{}", org.login),
                    domains: Vec::new(),
                    logins: users,
                    evidence: format!("GitHub organization @{}", org.login),
                });
            }
            None => discovery.review.push(ReviewItem {
                kind: "Unknown organization",
                subject: format!("@{login}"),
                detail: format!(
                    "{} in the profile of {}",
                    if organization.is_some() {
                        "no website on its GitHub profile"
                    } else {
                        "not a GitHub organization"
                    },
                    users.iter().map(|l| format!("@{l}")).collect::<Vec<_>>().join(", ")
                ),
            }),
        }
    }
    discovery.companies.sort_by(|a, b| a.id.cmp(&b.id));
    discovery.aliases.sort_by(|a, b| (&a.company_id, &a.alias).cmp(&(&b.company_id, &b.alias)));

    // 3. Per contributor: new affiliations, changes, conflicts, and history from domains.
    let today_date = format_date(today);
    let mut first_activity: HashMap<&str, i64> = HashMap::new();
    for commit in &state.commits {
        if let Some(at) = commit.authored_at.or(commit.committed_at) {
            let entry = first_activity.entry(&commit.author_contributor_id).or_insert(at.0);
            *entry = (*entry).min(at.0);
        }
    }
    for pr in &state.pull_requests {
        if let Some(at) = pr.created_at {
            let entry = first_activity.entry(&pr.author_contributor_id).or_insert(at.0);
            *entry = (*entry).min(at.0);
        }
    }
    let name =
        |id: &str| -> String { if id.is_empty() { "Independent".to_string() } else { catalog.company_name(id) } };
    let new_company_name = |id: &str| discovery_name(&discovery.companies, id).unwrap_or_else(|| name(id));
    let mut unmatched_profiles: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for contributor in &humans {
        let id = contributor.id.as_str();
        let profile = contributor.profile_company.trim();
        let profile_company: Option<String> = if profile.is_empty() {
            None
        } else if let Some(company) = catalog.aliases.get(&normalize_company(profile)) {
            Some(company.clone())
        } else if let Some(handle) = single_handle(profile, ignore) {
            // Multiple companies stay unmapped for review; one handle names the employer.
            catalog
                .organization_company(&handle)
                .map(str::to_string)
                .or_else(|| new_orgs.get(&handle.to_lowercase()).cloned())
        } else {
            if !ignore.profile_companies.contains(&normalize_company(profile)) {
                unmatched_profiles.entry(profile.to_string()).or_default().push(contributor.login.clone());
            }
            None
        };
        let org_companies: BTreeMap<String, String> = contributor
            .organizations
            .iter()
            .filter(|o| !ignore.organizations.contains(*o))
            .filter_map(|o| catalog.organization_company(o).map(|c| (c.to_string(), o.clone())))
            .collect();
        let mapped_runs: Vec<(String, &DomainRun)> = runs
            .get(id)
            .into_iter()
            .flatten()
            .filter_map(|run| domain_company(&run.domain).map(|company| (company, run)))
            .collect();
        let latest_domain = mapped_runs
            .iter()
            .max_by_key(|(_, run)| run.last)
            .filter(|(_, run)| day_of(run.last) >= today - CURRENT_DOMAIN_DAYS);
        let domain_evidence = |run: &DomainRun| {
            format!(
                "{} commits from {} ({} to {})",
                thousands(run.commits as u64),
                run.domain,
                date(run.first),
                date(run.last)
            )
        };
        let signal: Option<(String, String)> = if let Some(company) = &profile_company {
            Some((company.clone(), format!("profile company \"{profile}\"")))
        } else if org_companies.len() == 1 {
            let (company, org) = org_companies.iter().next().expect("one entry");
            Some((company.clone(), format!("public member of @{org}")))
        } else {
            latest_domain.map(|(company, run)| (company.clone(), domain_evidence(run)))
        };

        let undated = catalog.current.get(id);
        let periods = catalog.periods.get(id).cloned().unwrap_or_default();
        if undated.is_none() && periods.is_empty() {
            if let Some((company, evidence)) = &signal {
                discovery.affiliations.push(NewAffiliation {
                    login: contributor.login.clone(),
                    company_id: company.clone(),
                    evidence: format!("{}: {evidence}", profile_link(contributor)),
                });
            }
        } else {
            let effective = catalog.effective(id, &today_date).map(|a| a.company_id.clone()).unwrap_or_default();
            let mut changed = false;
            let map_profile =
                |value: &str| -> String { catalog.aliases.get(&normalize_company(value)).cloned().unwrap_or_default() };
            let first = first_activity.get(id).map(|t| date(*t)).unwrap_or_default();
            if let [.., previous, last] = contributor.profile_company_history.as_slice() {
                let (before, after) = (map_profile(&previous.company), map_profile(&last.company));
                if before != after && after != effective {
                    let since = last.observed_at.map(|t| date(t.0)).unwrap_or_default();
                    let until =
                        previous.last_observed_at.or(previous.observed_at).map(|t| date(t.0)).unwrap_or_default();
                    discovery.changes.push(Change {
                        login: contributor.login.clone(),
                        contributor_id: id.to_string(),
                        from: effective.clone(),
                        to: after,
                        date: since.clone(),
                        first_activity: first.clone(),
                        evidence: format!(
                            "{}: profile company changed from \"{}\" (last seen {until}) to \"{}\" (first seen {since})",
                            profile_link(contributor),
                            previous.company,
                            last.company
                        ),
                    });
                    changed = true;
                }
            }
            if !changed && let [.., previous, last] = contributor.organization_history.as_slice() {
                let companies = |orgs: &[String]| -> BTreeSet<String> {
                    orgs.iter()
                        .filter(|o| !ignore.organizations.contains(*o))
                        .filter_map(|o| catalog.organization_company(o).map(str::to_string))
                        .collect()
                };
                let (before, after) = (companies(&previous.organizations), companies(&last.organizations));
                let joined: Vec<&String> = after.difference(&before).collect();
                let left = before.contains(&effective) && !after.contains(&effective);
                let since = last.observed_at.map(|t| date(t.0)).unwrap_or_default();
                let target = match joined.as_slice() {
                    [company] if **company != effective => Some((*company).clone()),
                    [] if left && signal.as_ref().is_none_or(|(s, _)| *s != effective) => Some(String::new()),
                    _ => None,
                };
                if let Some(to) = target {
                    discovery.changes.push(Change {
                        login: contributor.login.clone(),
                        contributor_id: id.to_string(),
                        from: effective.clone(),
                        to: to.clone(),
                        date: since.clone(),
                        first_activity: first.clone(),
                        evidence: format!(
                            "{}: public organizations changed from {:?} to {:?} (first seen {since})",
                            profile_link(contributor),
                            previous.organizations,
                            last.organizations
                        ),
                    });
                    changed = true;
                }
            }
            if !changed
                && let Some((company, run)) = latest_domain
                && *company != effective
            {
                let previous_last = mapped_runs.iter().filter(|(c, _)| *c == effective).map(|(_, r)| r.last).max();
                if previous_last.is_some_and(|last| last <= run.first + DOMAIN_OVERLAP_DAYS * SECONDS_PER_DAY) {
                    discovery.changes.push(Change {
                        login: contributor.login.clone(),
                        contributor_id: id.to_string(),
                        from: effective.clone(),
                        to: company.clone(),
                        date: date(run.first),
                        first_activity: first.clone(),
                        evidence: format!("{}: {}", profile_link(contributor), domain_evidence(run)),
                    });
                    changed = true;
                }
            }
            if !changed
                && let Some((company, evidence)) = &signal
                && *company != effective
            {
                discovery.review.push(ReviewItem {
                    kind: "Conflicting signal",
                    subject: profile_link(contributor),
                    detail: format!("curated as {}, but {evidence} ({})", name(&effective), new_company_name(company)),
                });
            }
        }

        // Earlier affiliations from a clean switch of commit email domains.
        let mut merged: Vec<(String, i64, i64, usize, Vec<String>)> = Vec::new();
        for (company, run) in &mapped_runs {
            match merged.last_mut() {
                Some(last) if last.0 == *company => {
                    last.2 = last.2.max(run.last);
                    last.3 += run.commits;
                    last.4.push(run.domain.clone());
                }
                _ => merged.push((company.clone(), run.first, run.last, run.commits, vec![run.domain.clone()])),
            }
        }
        if merged.len() >= 2 {
            let clean = merged.windows(2).all(|pair| pair[0].2 <= pair[1].1 + DOMAIN_OVERLAP_DAYS * SECONDS_PER_DAY);
            let distinct: BTreeSet<&String> = merged.iter().map(|m| &m.0).collect();
            if clean && distinct.len() == merged.len() {
                for pair in merged.windows(2) {
                    let (company, first, last, commits, domains) = &pair[0];
                    let next = pair[1].1;
                    let end =
                        if next - *last <= DOMAIN_GAP_DAYS * SECONDS_PER_DAY { next } else { *last + SECONDS_PER_DAY };
                    let (valid_from, valid_until) = (date(*first), date(end));
                    if valid_from >= valid_until {
                        continue;
                    }
                    let covered = catalog.historical(id, &date((*first + end) / 2)) == *company;
                    let overlaps = periods.iter().any(|p| {
                        p.valid_from < valid_until && (p.valid_until.is_empty() || valid_from < p.valid_until)
                    });
                    if covered || overlaps {
                        continue;
                    }
                    discovery.periods.push(Period {
                        login: contributor.login.clone(),
                        company_id: company.clone(),
                        valid_from,
                        valid_until,
                        evidence: format!(
                            "{}: {} commits from {} until {}, then {}",
                            profile_link(contributor),
                            thousands(*commits as u64),
                            domains.join(", "),
                            date(*last),
                            pair[1].4.join(", ")
                        ),
                    });
                }
            }
        }
    }
    for (profile, logins) in unmatched_profiles {
        discovery.review.push(ReviewItem {
            kind: "Unmatched profile company",
            subject: format!("\"{profile}\""),
            detail: logins.iter().map(|l| format!("@{l}")).collect::<Vec<_>>().join(", "),
        });
    }
    discovery
}

fn discovery_name(companies: &[NewCompany], id: &str) -> Option<String> {
    companies.iter().find(|c| c.id == id).map(|c| c.canonical_name.clone())
}

/// The review report, for the job summary and the pull request description.
pub fn markdown(discovery: &Discovery, catalog: &CompanyCatalog, applied: Option<&str>) -> String {
    let name = |id: &str| -> String {
        if id.is_empty() {
            "Independent".to_string()
        } else {
            discovery_name(&discovery.companies, id).unwrap_or_else(|| catalog.company_name(id))
        }
    };
    let mut out = String::new();
    let _ = writeln!(out, "## Company and affiliation discovery\n");
    if let Some(note) = applied {
        let _ = writeln!(out, "{note}\n");
    }
    if !discovery.has_edits() && discovery.review.is_empty() {
        let _ = writeln!(out, "No new companies, affiliations, or affiliation changes found.\n");
        return out;
    }
    if !discovery.companies.is_empty() {
        let _ = writeln!(
            out,
            "### New companies\n\n| Company | Website | Alias | Domains | Evidence |\n|---|---|---|---|---|"
        );
        for c in &discovery.companies {
            let named = if c.logins.is_empty() {
                String::new()
            } else {
                format!("; named by {}", c.logins.iter().map(|l| format!("@{l}")).collect::<Vec<_>>().join(", "))
            };
            let _ = writeln!(
                out,
                "| {} (`{}`) | {} | `{}` | {} | {}{named} |",
                c.canonical_name,
                c.id,
                c.website_url,
                c.alias,
                c.domains.join(", "),
                c.evidence
            );
        }
        out.push('\n');
    }
    if !discovery.aliases.is_empty() {
        let _ = writeln!(out, "### Company aliases\n\n| Alias | Company | Named by |\n|---|---|---|");
        for a in &discovery.aliases {
            let _ = writeln!(
                out,
                "| `{}` | {} | {} |",
                a.alias,
                name(&a.company_id),
                a.logins.iter().map(|l| format!("@{l}")).collect::<Vec<_>>().join(", ")
            );
        }
        out.push('\n');
    }
    if !discovery.domains.is_empty() {
        let _ = writeln!(out, "### Email domains\n\n| Domain | Company | Evidence |\n|---|---|---|");
        for d in &discovery.domains {
            let _ = writeln!(out, "| `{}` | {} | {} |", d.domain, name(&d.company_id), d.evidence);
        }
        out.push('\n');
    }
    if !discovery.affiliations.is_empty() {
        let _ = writeln!(out, "### New affiliations\n\n| Contributor | Company | Evidence |\n|---|---|---|");
        for a in &discovery.affiliations {
            let (link, evidence) = a.evidence.split_once(": ").unwrap_or((&a.login, &a.evidence));
            let _ = writeln!(out, "| {link} | {} | {evidence} |", name(&a.company_id));
        }
        out.push('\n');
    }
    if !discovery.changes.is_empty() {
        let _ = writeln!(
            out,
            "### Affiliation changes\n\n| Contributor | From | To | Since | Evidence |\n|---|---|---|---|---|"
        );
        for c in &discovery.changes {
            let (link, evidence) = c.evidence.split_once(": ").unwrap_or((&c.login, &c.evidence));
            let to = if c.to.is_empty() { "unknown (left)".to_string() } else { name(&c.to) };
            let _ = writeln!(out, "| {link} | {} | {to} | {} | {evidence} |", name(&c.from), c.date);
        }
        out.push('\n');
    }
    if !discovery.periods.is_empty() {
        let _ = writeln!(
            out,
            "### Earlier affiliations from commit email domains\n\n| Contributor | Company | From | Until (exclusive) | Evidence |\n|---|---|---|---|---|"
        );
        for p in &discovery.periods {
            let (link, evidence) = p.evidence.split_once(": ").unwrap_or((&p.login, &p.evidence));
            let _ = writeln!(
                out,
                "| {link} | {} | {} | {} | {evidence} |",
                name(&p.company_id),
                p.valid_from,
                p.valid_until
            );
        }
        out.push('\n');
    }
    if !discovery.review.is_empty() {
        let _ = writeln!(
            out,
            "### Needs a person\n\nNot edited automatically.\n\n| Kind | Subject | Detail |\n|---|---|---|"
        );
        for r in &discovery.review {
            let _ = writeln!(out, "| {} | {} | {} |", r.kind, r.subject, r.detail);
        }
        out.push('\n');
    }
    let _ = writeln!(
        out,
        "Dates come from public profile observations and commit email domains; they bound when a change was \
         seen, not exact employment dates. Add values to `config/discovery.yaml` to stop proposing them."
    );
    out
}

#[cfg(test)]
mod tests;
