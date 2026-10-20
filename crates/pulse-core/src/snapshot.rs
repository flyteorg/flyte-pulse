//! The precomputed, compact dataset the browser downloads (postcard encoded).
//!
//! The site and the WebAssembly module are built and deployed together, so the
//! format is versioned by a magic header instead of staying backward compatible.
//! Index fields refer to positions in the snapshot tables; [`NONE`] marks a
//! missing reference.

use serde::{Deserialize, Serialize};

use crate::state::{Language, TargetBranch};

pub const MAGIC: &[u8; 8] = b"PULSE\x00\x00\x02";
pub const NONE: u32 = u32::MAX;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub meta: Meta,
    pub repositories: Vec<Repo>,
    pub contributors: Vec<Person>,
    /// Metric participants without a contributor row, such as `ai-model:<key>`
    /// and `ai-provider:<name>`. Key index `contributors.len() + i` refers to `extra_keys[i]`.
    pub extra_keys: Vec<String>,
    /// Target branch names; index 0 is the empty string (no branch).
    pub branches: Vec<String>,
    pub companies: Vec<Company>,
    pub metrics: Vec<Metric>,
    pub company_metrics: Vec<CompanyMetric>,
    pub company_coverage: Vec<Coverage>,
    pub ai_identities: Vec<AiIdentity>,
    pub pull_requests: Vec<Pr>,
    pub commits: Vec<CommitRecord>,
    /// (pull request index, commit index)
    pub pr_commits: Vec<(u32, u32)>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Meta {
    pub version: String,
    /// Unix seconds when the snapshot was built.
    pub generated_at: i64,
    /// Unix seconds when the collected state was last updated.
    pub raw_generated_at: i64,
    pub raw_version: String,
    pub catalog_revision: String,
    pub mapping_revision: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Repo {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub module: String,
    pub status: String,
    pub target_branches: Vec<TargetBranch>,
    pub languages: Vec<Language>,
}

impl Repo {
    pub fn full_name(&self) -> String {
        if self.owner.is_empty() && self.name.is_empty() {
            self.id.clone()
        } else {
            format!("{}/{}", self.owner, self.name)
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Person {
    pub id: String,
    pub login: String,
    pub display_name: String,
    pub profile_url: String,
    pub kind: String,
    pub kind_source: String,
    pub profile_company: String,
    pub is_bot: bool,
    /// Current affiliation at build time (profile match or curated mapping).
    pub affiliation: Affiliation,
    /// Curated dated periods.
    pub affiliations: Vec<Affiliation>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Affiliation {
    pub contributor_id: String,
    pub company_id: String,
    pub company_name: String,
    pub website_url: String,
    pub status: String,
    pub source: String,
    pub reviewed: bool,
    pub observed_at: String,
    pub evidence: String,
    pub valid_from: String,
    pub valid_until: String,
    pub conflict: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Company {
    pub id: String,
    pub canonical_name: String,
    pub aliases: Vec<String>,
    pub website_url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum MetricKind {
    PullRequestsCreated,
    PullRequestsMerged,
    Commits,
    Additions,
    Deletions,
    SharedAdditions,
    SharedDeletions,
}

/// Daily activity per repository, contributor key, metric, and optional branch.
/// Rows with `branch == 0` are repository-wide; branch rows repeat the same
/// activity scoped to one configured target branch.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Metric {
    pub date: i32,
    pub repo: u32,
    pub key: u32,
    pub kind: MetricKind,
    pub branch: u32,
    pub value: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum CompanyMetricKind {
    Commits,
    PullRequestsOpened,
    PullRequestsMerged,
    SharedAdditions,
    SharedDeletions,
}

impl CompanyMetricKind {
    pub const ALL: [CompanyMetricKind; 5] = [
        Self::Commits,
        Self::PullRequestsOpened,
        Self::PullRequestsMerged,
        Self::SharedAdditions,
        Self::SharedDeletions,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Commits => "commits",
            Self::PullRequestsOpened => "pull_requests_opened",
            Self::PullRequestsMerged => "pull_requests_merged",
            Self::SharedAdditions => "shared_additions",
            Self::SharedDeletions => "shared_deletions",
        }
    }
}

/// Deduplicated company credit. `company` indexes `companies`; `companies.len()`
/// means Independent.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct CompanyMetric {
    pub date: i32,
    pub repo: u32,
    pub branch: u32,
    pub company: u32,
    pub kind: CompanyMetricKind,
    pub value: i64,
}

/// Events in scope versus events whose credited people all have curated mappings.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Coverage {
    pub date: i32,
    pub repo: u32,
    pub branch: u32,
    pub kind: CompanyMetricKind,
    pub value: i64,
    pub reviewed: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AiIdentity {
    pub key: String,
    pub provider: String,
    pub model: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Pr {
    pub repo: u32,
    pub provider_id: String,
    pub number: u64,
    /// Contributor key index of the author.
    pub author: u32,
    pub title: String,
    pub state: String,
    pub created_at: Option<i64>,
    pub merged_at: Option<i64>,
    pub branch: u32,
    pub additions: i64,
    pub deletions: i64,
    pub commit_count: u64,
    /// AI identity of an author classified as AI (identity kind), else NONE.
    pub author_ai_identity: u32,
    /// AI identity if the author is AI by kind or by name, else NONE.
    pub author_provider_identity: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AiName {
    /// The trailer's display name.
    pub name: String,
    /// Contributor key index of a matching AI contributor, else NONE.
    pub contributor: u32,
    pub identity: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Trailer {
    pub co_authored: bool,
    /// Contributor key index the trailer resolves to, else NONE.
    pub resolved: u32,
    /// AI identity the trailer names, else NONE.
    pub ai_identity: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CommitAi {
    /// The commit author's identity kind is AI.
    pub ai_author: bool,
    pub ai_author_identity: u32,
    /// AI identity if the author is AI by kind or by name, else NONE.
    pub author_provider_identity: u32,
    pub coauthors: Vec<AiName>,
    pub assistants: Vec<AiName>,
    pub has_coauthor_trailer: bool,
    pub has_assisted_trailer: bool,
    pub trailers: Vec<Trailer>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CommitRecord {
    pub repo: u32,
    pub sha: [u8; 20],
    pub author: u32,
    pub committer: u32,
    /// Git names, kept only when the identity has no contributor row.
    pub author_name: String,
    pub committer_name: String,
    pub committed_at: Option<i64>,
    pub subject: String,
    /// Present when the commit has an AI author, an AI-like author name, or trailers.
    pub ai: Option<Box<CommitAi>>,
}

impl CommitRecord {
    pub fn sha_hex(&self) -> String {
        self.sha.iter().map(|b| format!("{b:02x}")).collect()
    }
}

pub fn parse_sha(value: &str) -> Option<[u8; 20]> {
    if value.len() != 40 {
        return None;
    }
    let mut sha = [0u8; 20];
    for (index, byte) in sha.iter_mut().enumerate() {
        *byte = u8::from_str_radix(value.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(sha)
}

#[derive(Debug)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DecodeError {}

impl Snapshot {
    pub fn encode(&self) -> Result<Vec<u8>, DecodeError> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend(postcard::to_allocvec(self).map_err(|e| DecodeError(e.to_string()))?);
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let payload = bytes
            .strip_prefix(MAGIC.as_slice())
            .ok_or_else(|| DecodeError("not a Pulse snapshot, or built for a different version of the site".into()))?;
        postcard::from_bytes(payload).map_err(|e| DecodeError(format!("invalid snapshot: {e}")))
    }
}
