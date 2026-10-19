//! GitHub GraphQL client with rate-limit handling and the collector's queries.

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use pulse_core::time::parse_timestamp;
use serde::Deserialize;
use serde_json::{Value, json};

use super::identity::Account;

const GRAPHQL_URL: &str = "https://api.github.com/graphql";
const USER_AGENT: &str = "pulse-sync/0.1";
/// Pause until the reset when fewer points than this remain.
const LOW_REMAINING: i64 = 50;
const MAX_TRANSIENT_RETRIES: u32 = 4;
const MAX_SECONDARY_RETRIES: u32 = 5;
pub const COMMITS_PER_QUERY: usize = 100;
pub const USERS_PER_QUERY: usize = 50;

/// Raised when waiting for rate limits would exceed the sync's wait budget.
#[derive(Debug)]
pub struct WaitBudgetExhausted(pub String);

impl std::fmt::Display for WaitBudgetExhausted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GitHub wait budget exhausted: {}", self.0)
    }
}

impl std::error::Error for WaitBudgetExhausted {}

/// Commit SHA to its (author, committer) GitHub accounts.
pub type CommitAccounts = HashMap<String, (Option<Account>, Option<Account>)>;

pub struct Response {
    pub data: Value,
    pub errors: Vec<Value>,
}

impl Response {
    /// Fail on GraphQL errors, optionally tolerating NOT_FOUND (e.g. a renamed login).
    pub fn check(&self, allow_not_found: bool) -> Result<()> {
        let fatal: Vec<String> = self
            .errors
            .iter()
            .filter(|e| !(allow_not_found && e["type"] == "NOT_FOUND"))
            .map(|e| e["message"].as_str().unwrap_or("unknown error").to_string())
            .collect();
        if fatal.is_empty() { Ok(()) } else { bail!("GitHub GraphQL errors: {}", fatal.join("; ")) }
    }
}

pub struct GitHub {
    runtime: tokio::runtime::Runtime,
    http: reqwest::Client,
    token: String,
    pub requests: u64,
    pub points: u64,
    pub remaining: Option<i64>,
    reset_at: Option<i64>,
    waited: Duration,
    max_wait: Duration,
}

fn now_epoch() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn reset_delay(reset_at: Option<i64>) -> Duration {
    match reset_at {
        Some(reset) => Duration::from_secs((reset - now_epoch()).max(0) as u64 + 5),
        None => Duration::from_secs(60),
    }
}

impl GitHub {
    pub fn new(token: String, max_wait: Duration) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        let http = reqwest::Client::builder().user_agent(USER_AGENT).timeout(Duration::from_secs(90)).build()?;
        Ok(Self {
            runtime,
            http,
            token,
            requests: 0,
            points: 0,
            remaining: None,
            reset_at: None,
            waited: Duration::ZERO,
            max_wait,
        })
    }

    fn wait(&mut self, delay: Duration, reason: &str) -> Result<()> {
        if self.waited + delay > self.max_wait {
            return Err(WaitBudgetExhausted(format!(
                "already waited {}s; next wait would be {}s ({reason})",
                self.waited.as_secs(),
                delay.as_secs()
            ))
            .into());
        }
        eprintln!("  GitHub: waiting {}s ({reason})", delay.as_secs());
        std::thread::sleep(delay);
        self.waited += delay;
        Ok(())
    }

    pub fn query(&mut self, query: &str, variables: Value) -> Result<Response> {
        let body = json!({ "query": query, "variables": variables });
        let (mut transient, mut secondary) = (0u32, 0u32);
        loop {
            let result = self.runtime.block_on(async {
                let response = self.http.post(GRAPHQL_URL).bearer_auth(&self.token).json(&body).send().await?;
                let status = response.status().as_u16();
                let headers = response.headers().clone();
                let text = response.text().await?;
                Ok::<_, reqwest::Error>((status, headers, text))
            });
            let (status, headers, text) = match result {
                Ok(value) => value,
                Err(error) => {
                    if transient >= MAX_TRANSIENT_RETRIES {
                        return Err(anyhow!(error).context("GitHub GraphQL request failed"));
                    }
                    transient += 1;
                    self.wait(Duration::from_secs(2u64.pow(transient)), &format!("network error: {error}"))?;
                    continue;
                }
            };
            self.requests += 1;
            let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
            let header_remaining = header("x-ratelimit-remaining").and_then(|v| v.parse::<i64>().ok());
            let header_reset = header("x-ratelimit-reset").and_then(|v| v.parse::<i64>().ok());

            if matches!(status, 403 | 429) {
                if header_remaining == Some(0) {
                    self.wait(reset_delay(header_reset), "primary rate limit")?;
                    continue;
                }
                let retry_after = header("retry-after").and_then(|v| v.parse::<u64>().ok());
                let lower = text.to_lowercase();
                if status == 429
                    || retry_after.is_some()
                    || lower.contains("secondary rate limit")
                    || lower.contains("abuse")
                {
                    if secondary >= MAX_SECONDARY_RETRIES {
                        bail!("GitHub secondary rate limit persisted after {secondary} retries");
                    }
                    let delay = retry_after.unwrap_or(0).max(60 * 2u64.pow(secondary));
                    secondary += 1;
                    self.wait(Duration::from_secs(delay), "secondary rate limit")?;
                    continue;
                }
            }
            if matches!(status, 500 | 502 | 503 | 504) {
                if transient >= MAX_TRANSIENT_RETRIES {
                    bail!("GitHub GraphQL returned HTTP {status} after {transient} retries");
                }
                transient += 1;
                self.wait(Duration::from_secs(2u64.pow(transient)), &format!("HTTP {status}"))?;
                continue;
            }
            if status == 401 {
                bail!("GitHub rejected the token (HTTP 401); check PULSE_GITHUB_TOKEN or GITHUB_TOKEN");
            }
            if !(200..300).contains(&status) {
                bail!("GitHub GraphQL returned HTTP {status}: {}", &text[..text.len().min(500)]);
            }

            let payload: Value = serde_json::from_str(&text).context("GitHub GraphQL returned invalid JSON")?;
            let errors = payload["errors"].as_array().cloned().unwrap_or_default();
            if errors.iter().any(|e| e["type"] == "RATE_LIMITED") {
                self.wait(reset_delay(header_reset), "GraphQL rate limit")?;
                continue;
            }
            let data = payload["data"].clone();
            if data.is_null() && !errors.is_empty() {
                let message = errors[0]["message"].as_str().unwrap_or_default().to_lowercase();
                if (message.contains("something went wrong") || message.contains("timeout"))
                    && transient < MAX_TRANSIENT_RETRIES
                {
                    transient += 1;
                    self.wait(Duration::from_secs(2u64.pow(transient)), "GraphQL timeout")?;
                    continue;
                }
            }
            let rate = &data["rateLimit"];
            if rate.is_object() {
                self.points += rate["cost"].as_u64().unwrap_or(0);
                self.remaining = rate["remaining"].as_i64();
                self.reset_at = rate["resetAt"].as_str().and_then(parse_timestamp);
            } else if header_remaining.is_some() {
                self.remaining = header_remaining;
                self.reset_at = header_reset;
            }
            if self.remaining.is_some_and(|r| r < LOW_REMAINING) {
                let delay = reset_delay(self.reset_at);
                self.wait(delay, "rate limit nearly exhausted")?;
                self.remaining = None;
            }
            return Ok(Response { data, errors });
        }
    }

    pub fn repository_node_id(&mut self, owner: &str, name: &str) -> Result<String> {
        let response = self.query(
            "query($owner:String!,$name:String!){rateLimit{cost remaining resetAt} repository(owner:$owner,name:$name){id}}",
            json!({ "owner": owner, "name": name }),
        )?;
        response.check(false)?;
        response.data["repository"]["id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("repository {owner}/{name} was not found"))
    }

    pub fn pull_request_page(
        &mut self,
        owner: &str,
        name: &str,
        base: &str,
        after: Option<&str>,
        first: usize,
    ) -> Result<PullRequestPage> {
        const QUERY: &str = r#"
query($owner:String!,$name:String!,$base:String!,$after:String,$first:Int!){
  rateLimit{cost remaining resetAt}
  repository(owner:$owner,name:$name){
    pullRequests(baseRefName:$base,orderBy:{field:UPDATED_AT,direction:DESC},first:$first,after:$after){
      pageInfo{hasNextPage endCursor}
      nodes{
        fullDatabaseId databaseId number title state url
        createdAt updatedAt closedAt mergedAt
        baseRefName headRefOid mergeCommit{oid}
        additions deletions commits{totalCount}
        author{__typename login url ... on User{databaseId name company} ... on Bot{databaseId}}
      }
    }
  }
}"#;
        let response =
            self.query(QUERY, json!({ "owner": owner, "name": name, "base": base, "after": after, "first": first }))?;
        response.check(false)?;
        let connection = &response.data["repository"]["pullRequests"];
        if connection.is_null() {
            bail!("repository {owner}/{name} was not found");
        }
        serde_json::from_value(connection.clone()).context("unexpected pull request payload")
    }

    /// Commit author/committer accounts, keyed by commit SHA.
    pub fn commit_accounts(&mut self, owner: &str, name: &str, shas: &[String]) -> Result<CommitAccounts> {
        let mut result = HashMap::new();
        for batch in shas.chunks(COMMITS_PER_QUERY) {
            let mut fields = String::new();
            for (index, sha) in batch.iter().enumerate() {
                if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                    continue;
                }
                fields.push_str(&format!(
                    "c{index}:object(oid:\"{sha}\"){{... on Commit{{author{{user{{databaseId login name company url}}}} committer{{user{{databaseId login name company url}}}}}}}}\n"
                ));
            }
            if fields.is_empty() {
                continue;
            }
            let query = format!(
                "query($owner:String!,$name:String!){{rateLimit{{cost remaining resetAt}} repository(owner:$owner,name:$name){{\n{fields}}}}}"
            );
            let response = self.query(&query, json!({ "owner": owner, "name": name }))?;
            response.check(true)?;
            let repository = &response.data["repository"];
            for (index, sha) in batch.iter().enumerate() {
                let node = &repository[format!("c{index}")];
                if node.is_null() {
                    continue;
                }
                result.insert(
                    sha.clone(),
                    (user_account(&node["author"]["user"]), user_account(&node["committer"]["user"])),
                );
            }
        }
        Ok(result)
    }

    /// Public profiles by login; `None` when the login no longer exists.
    pub fn users(&mut self, logins: &[String]) -> Result<HashMap<String, Option<Account>>> {
        let mut result = HashMap::new();
        for batch in logins.chunks(USERS_PER_QUERY) {
            let mut fields = String::new();
            for (index, login) in batch.iter().enumerate() {
                fields.push_str(&format!(
                    "u{index}:user(login:{}){{databaseId login name company url organizations(first:50){{nodes{{login}}}}}}\n",
                    serde_json::to_string(login)?
                ));
            }
            let query = format!("query{{rateLimit{{cost remaining resetAt}}\n{fields}}}");
            let response = self.query(&query, json!({}))?;
            response.check(true)?;
            for (index, login) in batch.iter().enumerate() {
                result.insert(login.clone(), user_account(&response.data[format!("u{index}")]));
            }
        }
        Ok(result)
    }
}

/// Public details of a GitHub organization.
#[derive(Clone, Debug, Default)]
pub struct Organization {
    pub login: String,
    pub name: String,
    pub website_url: String,
}

impl GitHub {
    /// Organizations by login; `None` when the login is not an organization.
    pub fn organizations(&mut self, logins: &[String]) -> Result<HashMap<String, Option<Organization>>> {
        let mut result = HashMap::new();
        for batch in logins.chunks(USERS_PER_QUERY) {
            let mut fields = String::new();
            for (index, login) in batch.iter().enumerate() {
                fields.push_str(&format!(
                    "o{index}:organization(login:{}){{login name websiteUrl}}\n",
                    serde_json::to_string(login)?
                ));
            }
            let query = format!("query{{rateLimit{{cost remaining resetAt}}\n{fields}}}");
            let response = self.query(&query, json!({}))?;
            response.check(true)?;
            for (index, login) in batch.iter().enumerate() {
                let node = &response.data[format!("o{index}")];
                let organization = node["login"].as_str().map(|found| Organization {
                    login: found.to_string(),
                    name: node["name"].as_str().unwrap_or_default().to_string(),
                    website_url: node["websiteUrl"].as_str().unwrap_or_default().to_string(),
                });
                result.insert(login.clone(), organization);
            }
        }
        Ok(result)
    }
}

fn user_account(user: &Value) -> Option<Account> {
    let database_id = user["databaseId"].as_i64()?;
    Some(Account {
        database_id,
        login: user["login"].as_str().unwrap_or_default().to_string(),
        account_type: "User".into(),
        name: user["name"].as_str().map(str::to_string),
        company: user["company"].as_str().map(str::to_string),
        url: user["url"].as_str().map(str::to_string),
        organizations: user["organizations"]["nodes"]
            .as_array()
            .map(|nodes| nodes.iter().filter_map(|node| node["login"].as_str().map(str::to_string)).collect()),
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestPage {
    pub page_info: PageInfo,
    #[serde(default)]
    pub nodes: Vec<Option<PullRequestNode>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestNode {
    /// GraphQL BigInt, serialized as a string.
    pub full_database_id: Option<Value>,
    pub database_id: Option<i64>,
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub url: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub closed_at: Option<String>,
    pub merged_at: Option<String>,
    #[serde(default)]
    pub base_ref_name: String,
    pub head_ref_oid: Option<String>,
    pub merge_commit: Option<Oid>,
    pub additions: Option<i64>,
    pub deletions: Option<i64>,
    pub commits: Option<TotalCount>,
    pub author: Option<Actor>,
}

impl PullRequestNode {
    pub fn provider_id(&self) -> String {
        match &self.full_database_id {
            Some(Value::String(id)) if !id.is_empty() => id.clone(),
            Some(Value::Number(id)) => id.to_string(),
            _ => self.database_id.map(|id| id.to_string()).unwrap_or_default(),
        }
    }

    pub fn author_account(&self) -> Option<Account> {
        let author = self.author.as_ref()?;
        Some(Account {
            database_id: author.database_id?,
            login: author.login.clone(),
            account_type: author.typename.clone(),
            name: author.name.clone(),
            company: author.company.clone(),
            url: author.url.clone(),
            organizations: None,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct Oid {
    pub oid: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TotalCount {
    pub total_count: u64,
}

#[derive(Debug, Deserialize)]
pub struct Actor {
    #[serde(rename = "__typename")]
    pub typename: String,
    #[serde(default)]
    pub login: String,
    pub url: Option<String>,
    #[serde(rename = "databaseId")]
    pub database_id: Option<i64>,
    pub name: Option<String>,
    pub company: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_request_node_ids_and_authors() {
        let node: PullRequestNode = serde_json::from_value(json!({
            "fullDatabaseId": "2876543210", "databaseId": null, "number": 7, "title": "Fix", "state": "MERGED",
            "url": "https://github.com/o/r/pull/7", "createdAt": "2026-01-01T00:00:00Z",
            "updatedAt": "2026-01-02T00:00:00Z", "closedAt": null, "mergedAt": "2026-01-02T00:00:00Z",
            "baseRefName": "main", "headRefOid": "abc", "mergeCommit": {"oid": "def"},
            "additions": 3, "deletions": 1, "commits": {"totalCount": 2},
            "author": {"__typename": "Bot", "login": "dependabot", "url": "https://github.com/apps/dependabot", "databaseId": 49699333}
        }))
        .unwrap();
        assert_eq!(node.provider_id(), "2876543210");
        let author = node.author_account().unwrap();
        assert_eq!(author.contributor_id(), "github:49699333");
        assert_eq!(author.account_type, "Bot");
    }

    #[test]
    fn graphql_errors_respect_not_found() {
        let response = Response {
            data: Value::Null,
            errors: vec![json!({"type": "NOT_FOUND", "message": "Could not resolve to a User"})],
        };
        assert!(response.check(true).is_ok());
        assert!(response.check(false).is_err());
    }
}
