use std::collections::HashMap;
use std::path::Path;

use pulse_core::state::{Commit, CompanyObservation, Contributor, State};
use pulse_core::time::{Ts, parse_date, parse_timestamp};

use super::*;
use crate::companies::load_company_catalog;

const COMPANIES: &str = "\
# Reviewed companies.
companies:
- id: union
  canonical_name: Union
  website_url: https://union.ai
  aliases:
  - '@unionai'
- id: lyft
  canonical_name: Lyft
  website_url: https://www.lyft.com
  aliases: []
";

const AFFILIATIONS: &str = "\
# Reviewed affiliations.
affiliations:
- username: carol
  # Curated by hand.
  company_id: lyft
- username: erin
  company_id: union
- username: helper-bot
  company_id: lyft
";

fn ts(value: &str) -> Option<Ts> {
    parse_timestamp(value).map(Ts)
}

fn person(id: &str, login: &str, company: &str) -> Contributor {
    Contributor {
        id: id.into(),
        login: login.into(),
        display_name: login.into(),
        identity_kind: "HUMAN".into(),
        profile_company: company.into(),
        ..Contributor::default()
    }
}

fn commit(sha: char, author: &str, at: &str, domain: &str) -> Commit {
    Commit {
        repository_id: "github.com:R1".into(),
        sha: sha.to_string().repeat(40),
        author_contributor_id: author.into(),
        authored_at: ts(at),
        committed_at: ts(at),
        author_email_domain: domain.into(),
        ..Commit::default()
    }
}

fn setup() -> (tempfile::TempDir, State) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("companies.yaml"), COMPANIES).unwrap();
    std::fs::write(dir.path().join("affiliations.yaml"), AFFILIATIONS).unwrap();
    std::fs::write(dir.path().join("discovery.yaml"), "ignore:\n  profile_companies: [Student]\n").unwrap();
    let mut state = State::new();
    // alice: unaffiliated, profile names Union.
    state.contributors.push(person("github:1", "alice", "@unionai"));
    // bob: unaffiliated, commits from union.ai (maps by website).
    state.contributors.push(person("github:2", "bob", ""));
    // carol: curated Lyft, profile changed to Union.
    let mut carol = person("github:3", "carol", "Union");
    carol.profile_company_history = vec![
        CompanyObservation {
            company: "Lyft".into(),
            observed_at: ts("2026-01-01T00:00:00Z"),
            last_observed_at: ts("2026-03-01T00:00:00Z"),
        },
        CompanyObservation {
            company: "Union".into(),
            observed_at: ts("2026-03-08T00:00:00Z"),
            last_observed_at: ts("2026-10-01T00:00:00Z"),
        },
    ];
    state.contributors.push(carol);
    // dave: unaffiliated, profile names an unknown organization.
    state.contributors.push(person("github:4", "dave", "@acme-labs"));
    // erin: curated Union, commits from lyft.com before union.ai (an earlier affiliation).
    state.contributors.push(person("github:5", "erin", "Union"));
    state.contributors.push(person("github:6", "frank", "Student"));
    let mut bot = person("github:7", "helper-bot", "");
    bot.identity_kind = "BOT".into();
    bot.identity_kind_source = "CATALOG_RULE".into();
    state.contributors.push(bot);
    state.commits = vec![
        commit('a', "github:2", "2026-09-01T00:00:00Z", "union.ai"),
        commit('b', "github:3", "2025-06-01T00:00:00Z", ""),
        commit('c', "github:5", "2020-01-10T00:00:00Z", "lyft.com"),
        commit('d', "github:5", "2021-05-01T00:00:00Z", "lyft.com"),
        commit('e', "github:5", "2021-07-01T00:00:00Z", "union.ai"),
        commit('f', "github:5", "2026-09-01T00:00:00Z", "union.ai"),
    ];
    (dir, state)
}

fn run_discovery(dir: &Path, state: &State) -> (Discovery, CompanyCatalog) {
    let catalog = load_company_catalog(dir, &state.contributors).unwrap();
    let ignore = Ignore::load(dir).unwrap();
    assert_eq!(organization_candidates(state, &catalog, &ignore), vec!["acme-labs".to_string()]);
    let organizations = HashMap::from([(
        "acme-labs".to_string(),
        Some(Organization { login: "Acme-Labs".into(), name: "Acme Labs".into(), website_url: "acme.dev".into() }),
    )]);
    let today = parse_date("2026-10-18").unwrap();
    (discover(state, &catalog, &ignore, today, &organizations), catalog)
}

#[test]
fn finds_companies_affiliations_and_changes() {
    let (dir, state) = setup();
    let (discovery, catalog) = run_discovery(dir.path(), &state);

    let domains: Vec<(&str, &str)> =
        discovery.domains.iter().map(|d| (d.domain.as_str(), d.company_id.as_str())).collect();
    assert_eq!(domains, vec![("lyft.com", "lyft"), ("union.ai", "union")]);

    assert_eq!(discovery.companies.len(), 1);
    let acme = &discovery.companies[0];
    assert_eq!(
        (acme.id.as_str(), acme.website_url.as_str(), acme.alias.as_str()),
        ("acme-labs", "https://acme.dev", "@Acme-Labs")
    );

    let affiliations: Vec<(&str, &str)> =
        discovery.affiliations.iter().map(|a| (a.login.as_str(), a.company_id.as_str())).collect();
    assert_eq!(affiliations, vec![("alice", "union"), ("bob", "union"), ("dave", "acme-labs")]);

    assert_eq!(discovery.changes.len(), 1);
    let change = &discovery.changes[0];
    assert_eq!((change.login.as_str(), change.from.as_str(), change.to.as_str()), ("carol", "lyft", "union"));
    assert_eq!(change.date, "2026-03-08");
    assert_eq!(change.first_activity, "2025-06-01");

    let periods: Vec<(&str, &str, &str, &str)> = discovery
        .periods
        .iter()
        .map(|p| (p.login.as_str(), p.company_id.as_str(), p.valid_from.as_str(), p.valid_until.as_str()))
        .collect();
    assert_eq!(periods, vec![("erin", "lyft", "2020-01-10", "2021-07-01")]);
    let starts: Vec<(&str, &str, &str)> = discovery
        .current_starts
        .iter()
        .map(|s| (s.login.as_str(), s.company_id.as_str(), s.valid_from.as_str()))
        .collect();
    assert_eq!(starts, vec![("erin", "union", "2021-07-01")]);
    assert!(discovery.review.iter().all(|r| !r.subject.contains("Student")), "ignored strings are not proposed");
    assert!(
        discovery.review.iter().any(|r| r.kind == "Bot with an affiliation" && r.subject.contains("helper-bot")),
        "{:?}",
        discovery.review
    );

    let report = markdown(&discovery, &catalog, None);
    assert!(report.contains("### Affiliation changes"));
    assert!(report.contains("| [carol](https://github.com/carol) | Lyft | Union | 2026-03-08 |"));
    assert!(report.contains("### Start dates for current affiliations"));
    assert!(report.contains("| [erin](https://github.com/erin) | Union | 2021-07-01 |"));
    assert!(report.contains("profile company `@unionai`"), "{report}");
    // An "@" outside code spans would notify that account from the pull request.
    let outside_code: String = report.split('`').step_by(2).collect();
    assert!(!outside_code.contains('@'), "{report}");
}

#[test]
fn applies_edits_sorted_and_keeps_comments() {
    let (dir, state) = setup();
    let (discovery, _) = run_discovery(dir.path(), &state);
    apply(&discovery, dir.path(), &state.contributors).unwrap();

    let companies = std::fs::read_to_string(dir.path().join("companies.yaml")).unwrap();
    assert_eq!(
        companies,
        "\
# Reviewed companies.
companies:
- id: acme-labs
  canonical_name: 'Acme Labs'
  website_url: https://acme.dev
  aliases:
  - '@Acme-Labs'
- id: lyft
  canonical_name: Lyft
  website_url: https://www.lyft.com
  aliases: []
  domains:
  - lyft.com
- id: union
  canonical_name: Union
  website_url: https://union.ai
  aliases:
  - '@unionai'
  domains:
  - union.ai
"
    );

    // Changes and earlier periods date the current entries; comments stay inside their entry.
    let affiliations = std::fs::read_to_string(dir.path().join("affiliations.yaml")).unwrap();
    assert_eq!(
        affiliations,
        "\
# Reviewed affiliations.
affiliations:
- username: alice
  company_id: union
- username: bob
  company_id: union
- username: carol
  company_id: lyft
  valid_from: '2025-06-01'
  valid_until: '2026-03-08'
- username: carol
  # Curated by hand.
  company_id: union
  valid_from: '2026-03-08'
- username: dave
  company_id: acme-labs
- username: erin
  company_id: lyft
  valid_from: '2020-01-10'
  valid_until: '2021-07-01'
- username: erin
  company_id: union
  valid_from: '2021-07-01'
- username: helper-bot
  company_id: lyft
"
    );

    // The edited files load, and a second run finds nothing new for the applied entries.
    let catalog = load_company_catalog(dir.path(), &state.contributors).unwrap();
    assert_eq!(catalog.historical("github:3", "2026-01-01"), "lyft");
    assert_eq!(catalog.historical("github:3", "2026-04-01"), "union");
    assert_eq!(catalog.historical("github:5", "2020-06-01"), "lyft");
    assert_eq!(catalog.historical("github:5", "2026-09-01"), "union");
    assert_eq!(catalog.domain_company("union.ai"), Some("union"));
    let again = discover(
        &state,
        &catalog,
        &Ignore::load(dir.path()).unwrap(),
        parse_date("2026-10-18").unwrap(),
        &HashMap::new(),
    );
    assert!(!again.has_edits(), "{again:?}");
}
