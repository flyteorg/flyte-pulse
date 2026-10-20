//! End to end: a small state → snapshot → every dashboard route.

use std::path::Path;

use pulse::companies::load_company_catalog;
use pulse::snapshot_builder::build_snapshot;
use pulse_core::snapshot::Snapshot;
use pulse_core::state::{
    Commit, Contributor, Language, PullRequest, PullRequestCommit, Repository, State, TargetBranch,
};
use pulse_core::time::{Ts, day_of, parse_timestamp};
use pulse_core::views::Dataset;
use serde_json::Value;

fn ts(value: &str) -> Option<Ts> {
    parse_timestamp(value).map(Ts)
}

fn person(id: &str, login: &str, name: &str, kind: &str) -> Contributor {
    Contributor {
        id: id.into(),
        provider: "GITHUB".into(),
        login: login.into(),
        display_name: name.into(),
        identity_kind: kind.into(),
        identity_kind_source: "GITHUB_ACCOUNT_TYPE".into(),
        ..Contributor::default()
    }
}

fn state() -> State {
    let mut state = State::new();
    state.generated_at = ts("2026-10-10T00:00:00Z");
    state.repositories = vec![
        Repository {
            repository_id: "github.com:R1".into(),
            owner: "flyteorg".into(),
            name: "flyte".into(),
            module: "core".into(),
            status: "ACTIVE".into(),
            target_branches: vec![
                TargetBranch { name: "main".into(), label: "v2".into() },
                TargetBranch { name: "master".into(), label: "v1".into() },
            ],
            languages: vec![
                Language { name: "Go".into(), color: "#00ADD8".into(), bytes: 750 },
                Language { name: "Python".into(), color: "#3572A5".into(), bytes: 250 },
            ],
            ..Repository::default()
        },
        Repository {
            repository_id: "github.com:R2".into(),
            owner: "flyteorg".into(),
            name: "flytekit".into(),
            status: "ACTIVE".into(),
            target_branches: vec![TargetBranch { name: "master".into(), label: "v1".into() }],
            languages: vec![Language { name: "Python".into(), color: "#3572A5".into(), bytes: 1000 }],
            ..Repository::default()
        },
    ];
    state.contributors = vec![
        person("github:1", "alice", "Alice", "HUMAN"),
        person("github:2", "bob", "Bob", "HUMAN"),
        person("github:3", "dependabot[bot]", "dependabot[bot]", "BOT"),
    ];
    let pr = |repo: &str, id: &str, number: u64, author: &str, branch: &str, created: &str, merged: Option<&str>| {
        PullRequest {
            repository_id: repo.into(),
            provider_id: id.into(),
            number,
            author_contributor_id: author.into(),
            title: format!("PR {number}"),
            state: if merged.is_some() { "merged" } else { "open" }.into(),
            created_at: ts(created),
            merged_at: merged.and_then(ts),
            target_branch: branch.into(),
            additions: 10,
            deletions: 2,
            commit_count: 1,
            ..PullRequest::default()
        }
    };
    state.pull_requests = vec![
        pr("github.com:R1", "100", 1, "github:1", "main", "2026-10-01T10:00:00Z", Some("2026-10-02T10:00:00Z")),
        pr("github.com:R1", "101", 2, "github:2", "master", "2026-10-03T10:00:00Z", None),
        pr("github.com:R2", "200", 3, "github:3", "master", "2026-10-04T10:00:00Z", Some("2026-10-05T10:00:00Z")),
    ];
    let commit = |repo: &str, sha: char, author: &str, at: &str, message: &str| Commit {
        repository_id: repo.into(),
        sha: sha.to_string().repeat(40),
        author_contributor_id: author.into(),
        committer_contributor_id: author.into(),
        author_name: "Git Name".into(),
        committed_at: ts(at),
        message: message.into(),
        ..Commit::default()
    };
    state.commits = vec![
        commit(
            "github.com:R1",
            'a',
            "github:1",
            "2026-10-01T11:00:00Z",
            "Add feature\n\nCo-authored-by: Bob <2+bob@users.noreply.github.com>\nCo-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>",
        ),
        commit("github.com:R1", 'b', "github:2", "2026-10-03T11:00:00Z", "Fix\n\nAssisted-By: Cursor"),
        commit("github.com:R2", 'c', "github:3", "2026-10-04T11:00:00Z", "Bump deps"),
        // On a branch but not linked to a pull request: stored, not counted.
        commit("github.com:R2", 'd', "github:1", "2026-10-04T12:00:00Z", "Direct push"),
    ];
    let link = |repo: &str, pr: &str, sha: char| PullRequestCommit {
        repository_id: repo.into(),
        pull_request_id: pr.into(),
        commit_sha: sha.to_string().repeat(40),
    };
    state.pull_request_commits =
        vec![link("github.com:R1", "100", 'a'), link("github.com:R1", "101", 'b'), link("github.com:R2", "200", 'c')];
    state
}

fn dataset() -> Dataset {
    let state = state();
    let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let companies = load_company_catalog(&config, &state.contributors).unwrap();
    let snapshot = build_snapshot(&state, &companies, parse_timestamp("2026-10-10T12:00:00Z").unwrap());
    Dataset::new(Snapshot::decode(&snapshot.encode().unwrap()).unwrap())
}

fn today() -> i32 {
    day_of(parse_timestamp("2026-10-10T00:00:00Z").unwrap())
}

fn get(data: &Dataset, url: &str) -> Value {
    data.request(url, today()).unwrap_or_else(|e| panic!("{url}: {e:?}"))
}

#[test]
fn overview_counts_human_activity() {
    let data = dataset();
    let overview = get(&data, "/api/dashboard?days=30");
    let summary = &overview["summary"];
    assert_eq!(summary["pull_requests_opened"], 2, "bot PRs are excluded");
    assert_eq!(summary["pull_requests_merged"], 1);
    assert_eq!(summary["commits"], 2, "bot commits and unlinked commits are excluded");
    assert_eq!(summary["active_human_contributors"], 2);
    // flyte tracks two branches, so it appears once per branch.
    let names: Vec<&str> =
        overview["repository_activity"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"flyteorg/flyte · v2") && names.contains(&"flyteorg/flyte · v1"), "{names:?}");
    let bob = overview["human_top_contributors"].as_array().unwrap().iter().find(|c| c["id"] == "github:2").unwrap();
    assert_eq!(bob["shared_additions"], 10, "co-authors share the merged PR's lines");
    let ids: Vec<&str> =
        overview["top_contributors"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert!(ids.iter().all(|id| !id.starts_with("ai-")), "AI attribution keys are not contributors: {ids:?}");
}

#[test]
fn ai_attribution_from_trailers() {
    let data = dataset();
    let ai = get(&data, "/api/ai-usage");
    assert_eq!(ai["summary"]["commits_with_ai"], 2);
    assert_eq!(ai["summary"]["commits_ai_coauthored"], 1);
    assert_eq!(ai["summary"]["commits_ai_assisted"], 1);
    let providers: Vec<&str> =
        ai["top_ai_providers"].as_array().unwrap().iter().map(|p| p["provider"].as_str().unwrap()).collect();
    assert!(providers.contains(&"Claude") && providers.contains(&"Cursor"), "{providers:?}");
    let claude = get(&data, "/api/ai-contributors/provider%3Aclaude?provider=Claude&days=all");
    assert_eq!(claude["summary"]["commits_ai_coauthored"], 1);
    assert_eq!(claude["summary"]["shared_additions"], 10);
}

#[test]
fn language_breakdowns() {
    let data = dataset();
    let names = |value: &Value| -> Vec<(String, f64)> {
        value["languages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| (l["name"].as_str().unwrap().to_string(), l["share"].as_f64().unwrap()))
            .collect()
    };
    // Overview sums code size: 750 Go and 1,250 Python bytes.
    let overview = get(&data, "/api/dashboard?days=30");
    assert_eq!(names(&overview), [("Python".to_string(), 0.625), ("Go".to_string(), 0.375)]);
    let repo = get(&data, "/api/repositories/github.com%3AR1?days=365");
    assert_eq!(names(&repo), [("Go".to_string(), 0.75), ("Python".to_string(), 0.25)]);
    assert_eq!(repo["languages"][0]["color"], "#00ADD8");
    // Contributors get the languages of the repositories they committed to.
    let alice = get(&data, "/api/users/github%3A1?days=all");
    assert_eq!(names(&alice)[0].0, "Go");
}

#[test]
fn detail_pages_and_errors() {
    let data = dataset();
    let repo = get(&data, "/api/repositories/github.com%3AR1?days=365&target_branch=main");
    assert_eq!(repo["summary"]["pull_requests_merged"], 1);
    assert_eq!(repo["recent_pull_requests"][0]["url"], "https://github.com/flyteorg/flyte/pull/1");
    assert_eq!(get(&data, "/api/repositories")["repositories"].as_array().unwrap().len(), 3);

    let users = get(&data, "/api/users");
    assert_eq!(users["users"].as_array().unwrap().len(), 2);
    let alice = get(&data, "/api/users/github%3A1/history?days=all");
    assert_eq!(alice["summary"]["commits_authored"], 1);
    assert_eq!(alice["contributor"]["affiliation_history"][0]["company_name"], "Union");
    assert_eq!(alice["ai_usage"]["summary"]["commits_ai_coauthored"], 1);

    let companies = get(&data, "/api/companies?days=all");
    let rows = companies["companies"].as_array().unwrap();
    assert_eq!(rows[0]["id"], "independent", "Bob has no mapping, so his activity is Independent");
    assert_eq!(rows.iter().find(|c| c["id"] == "union").unwrap()["pull_requests_opened"], 1);
    let union = get(&data, "/api/companies/union?days=all");
    assert_eq!(union["summary"]["commits"], 1);
    assert_eq!(union["profile_matches"].as_array().unwrap().len(), 1);

    assert_eq!(data.request("/api/dashboard?days=12", today()).unwrap_err().status, 422);
    assert_eq!(data.request("/api/users/github%3A999", today()).unwrap_err().status, 404);
    assert_eq!(data.request("/api/companies/nope", today()).unwrap_err().status, 404);
    assert_eq!(data.request("/api/nope", today()).unwrap_err().status, 404);
}
