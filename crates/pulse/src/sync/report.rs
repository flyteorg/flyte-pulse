//! Markdown report of a sync run, for the GitHub Actions job summary.

use std::fmt::Write as _;

use pulse_core::state::State;

use super::RepoSummary;
use crate::report::{duration, thousands};

pub(super) struct SyncReport<'a> {
    pub repositories: usize,
    pub summaries: &'a [RepoSummary],
    pub failed: &'a [String],
    pub skipped: &'a [String],
    pub requests: u64,
    pub points: u64,
    pub remaining: Option<i64>,
    pub seconds: u64,
    pub state: &'a State,
}

pub(super) fn markdown(report: &SyncReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "## Sync\n");
    let quota = report.remaining.map(|left| format!(", {} left this hour", thousands(left))).unwrap_or_default();
    let _ = writeln!(
        out,
        "Synced **{}** of {} repositories in **{}** using {} GitHub requests ({} GraphQL points{quota}).\n",
        report.summaries.len(),
        report.repositories,
        duration(report.seconds),
        thousands(report.requests),
        thousands(report.points),
    );
    if !report.summaries.is_empty() {
        let _ = writeln!(
            out,
            "| Repository | Git | PRs updated | New PRs | Newly merged | New reviews | New commits | PRs relinked | New contributors | API points | Time |"
        );
        let _ = writeln!(out, "|---|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|");
        for s in report.summaries {
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                s.name,
                s.mode,
                thousands(s.prs_scanned as u64),
                thousands(s.new_pull_requests as u64),
                thousands(s.merged_pull_requests as u64),
                thousands(s.new_reviews as u64),
                thousands(s.new_commits as u64),
                thousands(s.prs_linked as u64),
                thousands(s.new_contributors as u64),
                thousands(s.api_points),
                duration(s.seconds),
            );
        }
        out.push('\n');
    }
    let rescanned: Vec<&str> = report.summaries.iter().filter(|s| s.prs_rescanned).map(|s| s.name.as_str()).collect();
    if !rescanned.is_empty() {
        let _ = writeln!(
            out,
            "> [!NOTE]\n> Read every pull request again to collect newly tracked fields (reviews): {}\n",
            rescanned.join(", ")
        );
    }
    let link_failures: usize = report.summaries.iter().map(|s| s.link_failures).sum();
    if link_failures > 0 {
        let _ = writeln!(out, "> [!WARNING]\n> Git could not list the commits of {link_failures} pull requests.\n");
    }
    if !report.failed.is_empty() {
        let _ = writeln!(out, "> [!CAUTION]\n> Failed, previous data kept: {}\n", report.failed.join(", "));
    }
    if !report.skipped.is_empty() {
        let _ = writeln!(
            out,
            "> [!WARNING]\n> Skipped after reaching the rate-limit wait budget: {}\n",
            report.skipped.join(", ")
        );
    }
    let state = report.state;
    let _ = writeln!(
        out,
        "State after sync: {} repositories, {} contributors, {} pull requests, {} reviews, {} commits, {} pull request commit links.\n",
        thousands(state.repositories.len() as u64),
        thousands(state.contributors.len() as u64),
        thousands(state.pull_requests.len() as u64),
        thousands(state.pull_request_reviews.len() as u64),
        thousands(state.commits.len() as u64),
        thousands(state.pull_request_commits.len() as u64),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_tables_and_problems() {
        let summaries = vec![RepoSummary {
            name: "flyteorg/flyte".into(),
            mode: "shallow",
            prs_rescanned: true,
            prs_scanned: 3,
            new_pull_requests: 1,
            merged_pull_requests: 2,
            new_reviews: 4,
            new_commits: 1200,
            api_points: 2,
            seconds: 75,
            ..RepoSummary::default()
        }];
        let report = SyncReport {
            repositories: 2,
            summaries: &summaries,
            failed: &["flyteorg/flytekit".into()],
            skipped: &[],
            requests: 11,
            points: 10,
            remaining: Some(4990),
            seconds: 185,
            state: &State::new(),
        };
        let text = markdown(&report);
        assert!(text.contains("Synced **1** of 2 repositories in **3m 5s**"));
        assert!(text.contains("4,990 left this hour"));
        assert!(text.contains("| flyteorg/flyte | shallow | 3 | 1 | 2 | 4 | 1,200 | 0 | 0 | 2 | 1m 15s |"));
        assert!(text.contains("newly tracked fields (reviews): flyteorg/flyte"));
        assert!(text.contains("Failed, previous data kept: flyteorg/flytekit"));
    }
}
