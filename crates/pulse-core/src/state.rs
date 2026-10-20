//! Collected facts and sync bookkeeping, persisted between runs as gzipped JSON.
//!
//! The tables mirror the raw Parquet dataset format, so such a dataset can seed the state.
//! JSON keeps the format readable and lets new optional fields default cleanly.

use serde::{Deserialize, Serialize};

use crate::time::Ts;

pub const STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub schema_version: u32,
    pub generated_at: Option<Ts>,
    pub catalog_revision: String,
    pub repositories: Vec<Repository>,
    pub contributors: Vec<Contributor>,
    pub pull_requests: Vec<PullRequest>,
    pub commits: Vec<Commit>,
    pub pull_request_commits: Vec<PullRequestCommit>,
    pub watermarks: Vec<Watermark>,
    /// Commit email to GitHub account mappings. Emails are stored only as hashes.
    pub email_identities: Vec<EmailIdentity>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TargetBranch {
    pub name: String,
    pub label: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Repository {
    /// `github.com:<GraphQL node ID>`
    pub repository_id: String,
    pub provider_id: String,
    pub host: String,
    pub owner: String,
    pub name: String,
    pub module: String,
    pub status: String,
    pub target_branches: Vec<TargetBranch>,
    /// GitHub's language breakdown of the default branch, largest first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<Language>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub languages_observed_at: Option<Ts>,
}

/// Bytes of code in one language, as GitHub's language detection reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Language {
    pub name: String,
    /// Hex color GitHub uses for the language, such as `#00ADD8`; may be empty.
    pub color: String,
    pub bytes: u64,
}

/// A run of identical profile company values: first seen at `observed_at`, last
/// confirmed at `last_observed_at`. A change happened between one run's last
/// confirmation and the next run's first observation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CompanyObservation {
    pub company: String,
    pub observed_at: Option<Ts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_observed_at: Option<Ts>,
}

/// A run of identical public organization memberships (lowercased logins).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OrganizationObservation {
    pub organizations: Vec<String>,
    pub observed_at: Option<Ts>,
    pub last_observed_at: Option<Ts>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Contributor {
    /// `github:<numeric account ID>`
    pub id: String,
    pub provider: String,
    pub provider_user_id: String,
    pub login: String,
    pub display_name: String,
    pub profile_url: String,
    pub is_bot: bool,
    pub profile_company: String,
    pub profile_observed_at: Option<Ts>,
    /// HUMAN, BOT, AI, or UNKNOWN
    pub identity_kind: String,
    pub identity_kind_source: String,
    pub profile_name_checked: bool,
    pub profile_company_history: Vec<CompanyObservation>,
    /// Public organization memberships at the last profile refresh (lowercased logins).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub organizations: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub organization_history: Vec<OrganizationObservation>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PullRequest {
    pub repository_id: String,
    /// GitHub's numeric pull request ID (REST `id`, GraphQL `fullDatabaseId`).
    pub provider_id: String,
    pub number: u64,
    pub author_contributor_id: String,
    pub title: String,
    /// open, closed, or merged
    pub state: String,
    pub created_at: Option<Ts>,
    pub updated_at: Option<Ts>,
    pub closed_at: Option<Ts>,
    pub merged_at: Option<Ts>,
    pub target_branch: String,
    pub head_sha: String,
    pub merge_commit_sha: String,
    pub additions: i64,
    pub deletions: i64,
    pub commit_count: u64,
    pub url: String,
    pub observed_at: Option<Ts>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Commit {
    pub repository_id: String,
    pub sha: String,
    pub author_contributor_id: String,
    pub committer_contributor_id: String,
    pub author_name: String,
    pub committer_name: String,
    /// Truncated SHA-256 of the lowercased author email; see `EmailIdentity`.
    pub author_email_hash: String,
    pub committer_email_hash: String,
    /// Domain of a corporate author email (free-mail and noreply domains are left
    /// empty); the address itself is never stored.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub author_email_domain: String,
    pub authored_at: Option<Ts>,
    pub committed_at: Option<Ts>,
    pub message: String,
    pub parent_shas: Vec<String>,
    pub additions: i64,
    pub deletions: i64,
    pub url: String,
    pub observed_at: Option<Ts>,
    /// Tracked target branches whose history contains this commit, from Git.
    pub branches: Vec<String>,
    /// Whether the details above were read from a Git clone.
    pub from_git: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct PullRequestCommit {
    pub repository_id: String,
    pub pull_request_id: String,
    pub commit_sha: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Watermark {
    pub repository_id: String,
    /// Start time of the last completed pull request scan.
    pub pr_cursor: Option<Ts>,
    /// Start time of the last completed Git scan. `None` means the next sync
    /// makes a full clone instead of a shallow fetch.
    pub git_cursor: Option<Ts>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EmailIdentity {
    pub email_hash: String,
    /// Empty when GitHub has no account for the email.
    pub contributor_id: String,
    pub checked_at: Option<Ts>,
}

impl State {
    pub fn new() -> Self {
        Self { schema_version: STATE_SCHEMA_VERSION, ..Self::default() }
    }

    pub fn watermark(&self, repository_id: &str) -> Option<&Watermark> {
        self.watermarks.iter().find(|w| w.repository_id == repository_id)
    }

    pub fn watermark_mut(&mut self, repository_id: &str) -> &mut Watermark {
        if let Some(index) = self.watermarks.iter().position(|w| w.repository_id == repository_id) {
            return &mut self.watermarks[index];
        }
        self.watermarks.push(Watermark { repository_id: repository_id.to_string(), ..Watermark::default() });
        self.watermarks.last_mut().expect("just pushed")
    }

    /// Sort every table by its natural key so serialized state diffs are stable.
    pub fn normalize(&mut self) {
        self.repositories.sort_by(|a, b| a.repository_id.cmp(&b.repository_id));
        self.contributors.sort_by(|a, b| a.id.cmp(&b.id));
        self.pull_requests.sort_by(|a, b| (&a.repository_id, &a.provider_id).cmp(&(&b.repository_id, &b.provider_id)));
        self.commits.sort_by(|a, b| (&a.repository_id, &a.sha).cmp(&(&b.repository_id, &b.sha)));
        self.pull_request_commits.sort();
        self.pull_request_commits.dedup();
        self.watermarks.sort_by(|a, b| a.repository_id.cmp(&b.repository_id));
        self.email_identities.sort_by(|a, b| a.email_hash.cmp(&b.email_hash));
    }
}
