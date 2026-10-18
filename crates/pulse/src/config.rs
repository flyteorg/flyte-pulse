//! The reviewed repository catalog (`config/repositories.yaml`).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use pulse_core::state::{Contributor, TargetBranch};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCatalog {
    #[serde(default)]
    identity_rules: Vec<RawIdentityRule>,
    #[serde(default)]
    repositories: Vec<RawRepository>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawIdentityRule {
    provider: String,
    login: String,
    kind: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRepository {
    provider: String,
    host: String,
    owner: String,
    name: String,
    #[serde(default)]
    module: String,
    status: String,
    #[serde(default)]
    provider_id: String,
    #[serde(default)]
    target_branches: Vec<RawBranch>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBranch {
    name: String,
    #[serde(default)]
    label: String,
}

#[derive(Clone, Debug)]
pub struct RepositorySpec {
    pub host: String,
    pub owner: String,
    pub name: String,
    pub module: String,
    /// ACTIVE or INACTIVE
    pub status: String,
    pub provider_id: String,
    pub target_branches: Vec<TargetBranch>,
}

impl RepositorySpec {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    pub fn is_active(&self) -> bool {
        self.status == "ACTIVE"
    }
}

#[derive(Clone, Debug)]
pub struct Catalog {
    /// SHA-256 of the catalog file.
    pub revision: String,
    /// Lowercased GitHub login to HUMAN, BOT, AI, or UNKNOWN.
    pub identity_rules: HashMap<String, String>,
    pub repositories: Vec<RepositorySpec>,
}

const KINDS: [&str; 4] = ["HUMAN", "BOT", "AI", "UNKNOWN"];

pub fn load_catalog(path: &Path) -> Result<Catalog> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot read catalog {}", path.display()))?;
    let raw: RawCatalog =
        serde_yaml_ng::from_slice(&bytes).with_context(|| format!("invalid catalog {}", path.display()))?;

    let mut identity_rules = HashMap::new();
    for (index, rule) in raw.identity_rules.iter().enumerate() {
        let context = format!("identity_rules[{index}]");
        if rule.provider.trim().to_uppercase() != "GITHUB" {
            bail!("{context}.provider must be GITHUB");
        }
        let login = rule.login.trim();
        if login.is_empty() {
            bail!("{context}.login must be a non-empty string");
        }
        let kind = rule.kind.trim().to_uppercase();
        if !KINDS.contains(&kind.as_str()) {
            bail!("{context}.kind must be HUMAN, BOT, AI, or UNKNOWN");
        }
        if identity_rules.insert(login.to_lowercase(), kind).is_some() {
            bail!("duplicate identity rule for GitHub login {login:?}");
        }
    }

    let mut repositories = Vec::new();
    let mut seen = HashSet::new();
    for (index, repository) in raw.repositories.iter().enumerate() {
        let context = format!("repositories[{index}]");
        if repository.provider.trim().to_uppercase() != "GITHUB" {
            bail!("{context}.provider must be GITHUB");
        }
        let status = repository.status.trim().to_uppercase();
        if status != "ACTIVE" && status != "INACTIVE" {
            bail!("{context}.status must be ACTIVE or INACTIVE");
        }
        let (host, owner, name) =
            (repository.host.trim().to_lowercase(), repository.owner.trim(), repository.name.trim());
        if host.is_empty() || owner.is_empty() || name.is_empty() {
            bail!("{context} must have host, owner, and name");
        }
        if host != "github.com" {
            bail!("{context}: only github.com repositories are supported, got {host:?}");
        }
        let mut branch_names = HashSet::new();
        let mut target_branches = Vec::new();
        for branch in &repository.target_branches {
            let branch_name = branch.name.trim();
            if branch_name.is_empty() {
                bail!("repository {owner}/{name} has an empty target branch name");
            }
            if !branch_names.insert(branch_name.to_string()) {
                bail!("duplicate target branch {branch_name:?} in repository {owner}/{name}");
            }
            target_branches
                .push(TargetBranch { name: branch_name.to_string(), label: branch.label.trim().to_string() });
        }
        if status == "ACTIVE" && target_branches.is_empty() {
            bail!("{context}.target_branches must list at least one branch for an ACTIVE repository");
        }
        if !seen.insert((host.clone(), owner.to_lowercase(), name.to_lowercase())) {
            bail!("duplicate repository {owner}/{name}");
        }
        repositories.push(RepositorySpec {
            host,
            owner: owner.to_string(),
            name: name.to_string(),
            module: repository.module.trim().to_string(),
            status,
            provider_id: repository.provider_id.trim().to_string(),
            target_branches,
        });
    }

    Ok(Catalog { revision: hex(&Sha256::digest(&bytes)), identity_rules, repositories })
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

const KNOWN_BOT_LOGINS: [&str; 3] = ["dependabot", "dependabot-preview", "github-actions"];

/// Identity kind and its evidence source for a GitHub account.
/// `account_type` is the GitHub type name (`User`, `Bot`, ...), empty when unknown.
pub fn classify_github_account(
    login: &str,
    account_type: &str,
    identity_rules: &HashMap<String, String>,
) -> (String, String) {
    let normalized = login.trim().to_lowercase();
    if normalized.is_empty() && account_type.is_empty() {
        return ("UNKNOWN".into(), "UNKNOWN".into());
    }
    if let Some(kind) = identity_rules.get(&normalized) {
        return (kind.clone(), "CATALOG_RULE".into());
    }
    let account_type = account_type.to_lowercase();
    if account_type == "bot" {
        return ("BOT".into(), "GITHUB_ACCOUNT_TYPE".into());
    }
    if normalized.ends_with("[bot]") || KNOWN_BOT_LOGINS.contains(&normalized.as_str()) {
        return ("BOT".into(), "BOT_LOGIN_HEURISTIC".into());
    }
    if account_type == "user" {
        return ("HUMAN".into(), "GITHUB_ACCOUNT_TYPE".into());
    }
    ("UNKNOWN".into(), "UNKNOWN".into())
}

/// Catalog rules override automatic classification.
pub fn apply_identity_rules(contributors: &mut [Contributor], identity_rules: &HashMap<String, String>) {
    for contributor in contributors {
        if let Some(kind) = identity_rules.get(&contributor.login.trim().to_lowercase()) {
            contributor.identity_kind = kind.clone();
            contributor.identity_kind_source = "CATALOG_RULE".into();
            if kind == "BOT" {
                contributor.is_bot = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_repository_catalog() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/repositories.yaml");
        let catalog = load_catalog(&path).unwrap();
        assert!(catalog.repositories.iter().any(|r| r.full_name() == "flyteorg/flyte"));
        let flyte = catalog.repositories.iter().find(|r| r.name == "flyte").unwrap();
        assert_eq!(flyte.target_branches.len(), 2);
        assert_eq!(catalog.identity_rules.get("goreleaserbot").map(String::as_str), Some("BOT"));
        assert_eq!(catalog.identity_rules.get("claude").map(String::as_str), Some("AI"));
    }

    #[test]
    fn identity_rules_win() {
        let mut contributors =
            vec![Contributor { login: "Claude".into(), identity_kind: "HUMAN".into(), ..Contributor::default() }];
        apply_identity_rules(&mut contributors, &HashMap::from([("claude".into(), "AI".into())]));
        assert_eq!(contributors[0].identity_kind, "AI");
        assert_eq!(contributors[0].identity_kind_source, "CATALOG_RULE");
        assert!(!contributors[0].is_bot);
    }

    #[test]
    fn classifies_accounts() {
        let rules = HashMap::from([("helper".to_string(), "AI".to_string())]);
        assert_eq!(classify_github_account("Helper", "User", &rules).0, "AI");
        assert_eq!(classify_github_account("dependabot[bot]", "Bot", &rules).0, "BOT");
        assert_eq!(classify_github_account("renovate[bot]", "", &rules).1, "BOT_LOGIN_HEURISTIC");
        assert_eq!(classify_github_account("octocat", "User", &rules).0, "HUMAN");
    }
}
