//! AI attribution from commit trailers and AI identities. Runs at build time;
//! the browser receives the results.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use pulse_core::state::Contributor;
use regex::Regex;

pub use pulse_core::AI_NAME_ALIASES;
pub use pulse_core::views::normalized_ai_name;

static AI_TRAILER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?im)^\s*(Co-Authored-By|Assisted-By)\s*:\s*(.*?)\s*$").expect("valid regex"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static PERSON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.*?)\s*<([^<>]+)>\s*$").unwrap());
static EMAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[\w.+-]+@[\w.-]+\.[A-Za-z]{2,}\b").unwrap());
static CONTEXT_NOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\s*\([^)]*(?:context|ctx|window)[^)]*\)").unwrap());
static CONTEXT_SIZE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b\d+\s*[kmg]\s*(?:context|ctx)(?:\s+window)?\b").unwrap());
static CLAUDE_PRODUCT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:(?:code|claude)\b\s*[-_:]*\s*)+").unwrap());
static NOREPLY_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+\+").unwrap());
/// A bare tool version left after the product name, such as the `2.1.81` of `Claude Code 2.1.81`.
static TOOL_VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+(?:\.\d+)+$").unwrap());
/// A dotted version ending a model name (`Opus 4.5`, `gpt-4.1`); group 1 is the major version.
static TRAILING_VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[\s_-])(\d+)\.\d+(?:\.\d+)*$").unwrap());

struct ToolAlias {
    pattern: Regex,
    key: &'static str,
    label: &'static str,
}

static TOOL_ALIASES: LazyLock<Vec<ToolAlias>> = LazyLock::new(|| {
    [
        (r"(?:(?:other)\s+)?cursor(?:\s+cursor)?(?:\s+ai)?", "cursor:cursor", "Cursor"),
        (r"(?:(?:other)\s+)?codex(?:\s+codex)?", "codex:codex", "Codex"),
        (
            r"(?:github\s+)?copilot(?:\s+autofix(?:\s+powered\s+by\s+ai)?|\s+powered\s+by\s+ai)?",
            "copilot:copilot",
            "Copilot",
        ),
    ]
    .into_iter()
    .map(|(pattern, key, label)| ToolAlias {
        pattern: Regex::new(&format!("(?i)^(?:{pattern})$")).unwrap(),
        key,
        label,
    })
    .collect()
});

struct Provider {
    detect: Regex,
    label: &'static str,
    strip: Regex,
}

static PROVIDERS: LazyLock<Vec<Provider>> = LazyLock::new(|| {
    [
        (r"^(?:anthropic[\s-]+)?claude(?:\b|$)", "Claude", r"^(?:anthropic[\s-]+)?claude\b"),
        (r"^(?:openai\s+)?gpt(?:[\s-]|$)", "OpenAI", r"^openai\s+"),
        (r"^(?:openai\s+)?chatgpt(?:[\s-]|$)", "OpenAI", r"^openai\s+"),
        (r"^(?:google\s+)?gemini(?:\s+|$)", "Google", r"^google\s+"),
        (r"^(?:github\s+)?copilot(?:\s+|$)", "GitHub", r"^github\s+"),
        (r"^deepseek(?:\s+|$)", "DeepSeek", r"^deepseek\b"),
        (r"^qwen(?:\s+|$)", "Qwen", r"^qwen\b"),
        (r"^(?:meta\s+)?llama(?:\s+|$)", "Meta", r"^(?:meta\s+)?"),
        (r"^mistral(?:\s+|$)", "Mistral", r"^mistral\b"),
        (r"^(?:xai\s+)?grok(?:\s+|$)", "xAI", r"^(?:xai\s+)?"),
    ]
    .into_iter()
    .map(|(detect, label, strip)| Provider {
        detect: Regex::new(&format!("(?i){detect}")).unwrap(),
        label,
        strip: Regex::new(&format!("(?i){strip}")).unwrap(),
    })
    .collect()
});

fn strip_separators(value: &str) -> &str {
    value.trim_matches(|c| matches!(c, ' ' | '-' | '_' | ':'))
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AiIdentity {
    pub key: String,
    pub provider: String,
    /// The model family and major version, such as `Opus 4`.
    pub model: String,
    pub name: String,
    /// The full version the attribution named, such as `4.5`; empty when it named
    /// none or only a major version.
    pub version: String,
}

/// Split an AI attribution into a stable provider and model identity. Point
/// releases of a model are grouped under the major version (`Opus 4.5` and
/// `Opus 4.6` are both `Opus 4`), with the exact version kept separately.
pub fn ai_model_identity(value: &str) -> AiIdentity {
    // `tool version / model`, such as `Claude Code 2.1.81 / Claude Opus 4.6`: the
    // last part naming a provider is the model.
    let value = value
        .split('/')
        .map(str::trim)
        .rfind(|part| PROVIDERS.iter().any(|p| p.detect.is_match(part)))
        .unwrap_or(value);
    let mut model_name = CONTEXT_NOTE.replace_all(value, "").into_owned();
    model_name = CONTEXT_SIZE.replace_all(&model_name, "").into_owned();
    model_name = strip_separators(SPACES.replace_all(&model_name, " ").as_ref()).to_string();

    for alias in TOOL_ALIASES.iter() {
        if alias.pattern.is_match(&model_name) {
            return AiIdentity {
                key: alias.key.to_string(),
                provider: alias.label.to_string(),
                model: String::new(),
                name: alias.label.to_string(),
                version: String::new(),
            };
        }
    }

    let mut provider = "Other";
    for candidate in PROVIDERS.iter() {
        if candidate.detect.is_match(&model_name) {
            provider = candidate.label;
            model_name = strip_separators(&candidate.strip.replacen(&model_name, 1, "")).to_string();
            break;
        }
    }
    // Claude Code is a product name; "Code" identifies the product, not a model.
    if provider == "Claude" {
        model_name = strip_separators(&CLAUDE_PRODUCT.replacen(&model_name, 1, "")).to_string();
    }
    if matches!(model_name.to_lowercase().as_str(), "" | "claude" | "code" | "unspecified" | "unspecified model")
        || TOOL_VERSION.is_match(&model_name)
    {
        return AiIdentity {
            key: format!("{}:unspecified model", provider.to_lowercase()),
            provider: provider.to_string(),
            model: String::new(),
            name: provider.to_string(),
            version: String::new(),
        };
    }
    let mut model_name = SPACES.replace_all(&model_name, " ").trim().to_string();
    let mut version = String::new();
    if let Some(captures) = TRAILING_VERSION.captures(&model_name)
        && let Some(major) = captures.get(1)
    {
        version = model_name[major.start()..].to_string();
        model_name = format!("{}{}", &model_name[..major.start()], major.as_str());
    }
    let model_name = if model_name.is_empty() { "Unspecified model".to_string() } else { model_name };
    let name = if provider == "Other" { model_name.clone() } else { format!("{provider} {model_name}") };
    AiIdentity {
        key: format!("{}:{}", provider.to_lowercase(), normalized_ai_name(&model_name)),
        provider: provider.to_string(),
        model: model_name,
        name,
        version,
    }
}

/// The display name in a trailer value such as `Name <email>`, never the email.
pub fn trailer_person(value: &str) -> String {
    let value = value.trim();
    match PERSON.captures(value) {
        Some(captures) => {
            let name = captures[1].trim();
            if name.is_empty() {
                captures[2].split('@').next().unwrap_or_default().to_string()
            } else {
                name.to_string()
            }
        }
        None => value.to_string(),
    }
}

/// Matchable labels for a trailer value without retaining or exposing the email.
pub fn trailer_identity_candidates(value: &str) -> Vec<String> {
    let value = value.trim();
    let Some(captures) = PERSON.captures(value) else {
        return if value.is_empty() { vec![] } else { vec![value.to_string()] };
    };
    let mut candidates = Vec::new();
    let name = captures[1].trim();
    if !name.is_empty() {
        candidates.push(name.to_string());
    }
    let email = &captures[2];
    let (local, domain) = email.split_once('@').unwrap_or((email, ""));
    let local = if domain.eq_ignore_ascii_case("users.noreply.github.com") {
        NOREPLY_ID.replace(local, "").into_owned()
    } else {
        local.to_string()
    };
    if !local.is_empty() {
        candidates.push(local);
    }
    candidates
}

/// A Co-Authored-By / Assisted-By trailer resolved against the known contributors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTrailer {
    pub role: TrailerRole,
    /// The display name in the trailer, never the address.
    pub person: String,
    /// The contributor ID the trailer resolves to, or empty.
    pub resolved: String,
    /// The name is a recognized AI tool.
    pub is_ai: bool,
}

/// Parse and resolve every trailer of a commit message once, for all consumers.
pub fn resolve_trailers(
    message: &str,
    aliases: &HashMap<String, HashSet<String>>,
    known_ai_names: &HashSet<String>,
) -> Vec<ResolvedTrailer> {
    trailers(message)
        .into_iter()
        .map(|(role, raw)| {
            let person = trailer_person(&raw);
            let is_ai = is_ai_name(&person, known_ai_names);
            ResolvedTrailer { role, resolved: resolve_trailer_contributor(&raw, aliases), person, is_ai }
        })
        .collect()
}

/// `(role, raw person)` pairs for each Co-Authored-By / Assisted-By trailer.
pub fn trailers(message: &str) -> Vec<(TrailerRole, String)> {
    AI_TRAILER
        .captures_iter(message)
        .map(|captures| {
            let role = if captures[1].eq_ignore_ascii_case("co-authored-by") {
                TrailerRole::CoAuthoredBy
            } else {
                TrailerRole::AssistedBy
            };
            (role, captures[2].to_string())
        })
        .collect()
}

/// Full trailer lines, used to keep attribution while dropping commit bodies.
pub fn trailer_lines(message: &str) -> Vec<String> {
    AI_TRAILER.find_iter(message).map(|m| m.as_str().to_string()).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrailerRole {
    CoAuthoredBy,
    AssistedBy,
}

/// First non-empty, non-trailer line with email addresses redacted.
pub fn commit_subject(message: &str) -> String {
    for line in message.lines() {
        if line.trim().is_empty() || AI_TRAILER.is_match(line) {
            continue;
        }
        return EMAIL.replace_all(line, "[email]").into_owned();
    }
    String::new()
}

/// Normalized display names and logins mapped to contributor IDs.
pub fn contributor_alias_index(contributors: &[Contributor]) -> HashMap<String, HashSet<String>> {
    let mut aliases: HashMap<String, HashSet<String>> = HashMap::new();
    for contributor in contributors {
        if contributor.id.is_empty() {
            continue;
        }
        for field in [&contributor.display_name, &contributor.login] {
            let alias = normalized_ai_name(field);
            if !alias.is_empty() {
                aliases.entry(alias).or_default().insert(contributor.id.clone());
            }
        }
    }
    aliases
}

pub fn resolve_trailer_contributor(value: &str, aliases: &HashMap<String, HashSet<String>>) -> String {
    for candidate in trailer_identity_candidates(value) {
        if let Some(matches) = aliases.get(&normalized_ai_name(&candidate))
            && matches.len() == 1
        {
            return matches.iter().next().cloned().unwrap_or_default();
        }
    }
    String::new()
}

pub fn ai_contributor_names(contributors: &[Contributor]) -> HashSet<String> {
    contributors
        .iter()
        .filter(|c| c.identity_kind.eq_ignore_ascii_case("AI"))
        .flat_map(|c| [normalized_ai_name(&c.display_name), normalized_ai_name(&c.login)])
        .filter(|name| !name.is_empty())
        .collect()
}

pub fn is_ai_name(value: &str, known_ai_names: &HashSet<String>) -> bool {
    let normalized = normalized_ai_name(value);
    if normalized.is_empty() {
        return false;
    }
    if known_ai_names.contains(&normalized) {
        return true;
    }
    let padded = format!(" {normalized} ");
    AI_NAME_ALIASES.iter().any(|alias| normalized == *alias || padded.contains(&format!(" {alias} ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(value: &str) -> (String, String, String, String) {
        let i = ai_model_identity(value);
        (i.key, i.provider, i.model, i.name)
    }

    #[test]
    fn model_identities() {
        assert_eq!(
            identity("Claude Opus 4.5 (1M context)"),
            ("claude:opus 4".into(), "Claude".into(), "Opus 4".into(), "Claude Opus 4".into())
        );
        assert_eq!(
            identity("Claude Code"),
            ("claude:unspecified model".into(), "Claude".into(), "".into(), "Claude".into())
        );
        assert_eq!(identity("Cursor Agent").1, "Other");
        assert_eq!(identity("cursor").0, "cursor:cursor");
        assert_eq!(identity("GitHub Copilot").0, "copilot:copilot");
        assert_eq!(identity("gpt-5-codex").1, "OpenAI");
        assert_eq!(identity("gpt-5-codex").2, "gpt-5-codex");
        assert_eq!(identity("Gemini 2.5 Pro").0, "google:gemini 2 5 pro", "only a trailing version is grouped");
        assert_eq!(identity("anthropic-claude-sonnet-4").2, "sonnet-4");
    }

    #[test]
    fn point_releases_group_under_the_major_version() {
        let version = |value: &str| ai_model_identity(value).version;
        assert_eq!(version("Claude Opus 4.5 (1M context)"), "4.5");
        assert_eq!(version("Claude Opus 5"), "");
        assert_eq!(identity("Claude Opus 5").2, "Opus 5");
        assert_eq!(identity("Claude Fable 5.1").2, "Fable 5");
        assert_eq!(identity("Claude Sonnet 4.6").0, identity("Claude Sonnet 4.5").0);
        assert_eq!(
            identity("gpt-4.1"),
            ("openai:gpt 4".into(), "OpenAI".into(), "gpt-4".into(), "OpenAI gpt-4".into())
        );
        assert_eq!(version("gpt-4.1"), "4.1");
        assert_eq!(version("Gemini 2.5 Pro"), "");
    }

    #[test]
    fn tool_versions_are_not_models() {
        let combined = ai_model_identity("Claude Code 2.1.81 / Claude Opus 4.6");
        assert_eq!((combined.key.as_str(), combined.version.as_str()), ("claude:opus 4", "4.6"));
        let noted = ai_model_identity("Claude Code 2.1.157 / Claude Opus 4.7 (1M context)");
        assert_eq!((noted.model.as_str(), noted.version.as_str()), ("Opus 4", "4.7"));
        assert_eq!(identity("Claude Code 2.1.81").0, "claude:unspecified model");
    }

    #[test]
    fn trailers_and_people() {
        let message =
            "Fix bug (#12)\n\nCo-authored-by: Jane <1+jane@users.noreply.github.com>\nAssisted-By: Claude Code\n";
        let found = trailers(message);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0, TrailerRole::CoAuthoredBy);
        assert_eq!(trailer_person(&found[0].1), "Jane");
        assert_eq!(trailer_identity_candidates(&found[0].1), vec!["Jane", "jane"]);
        assert_eq!(trailer_person("<bob@example.com>"), "bob");
        assert_eq!(commit_subject("\nMail me@example.com now\nCo-authored-by: x"), "Mail [email] now");
    }

    #[test]
    fn ai_names() {
        let known = HashSet::from(["helper bot".to_string()]);
        assert!(is_ai_name("Claude", &known));
        assert!(is_ai_name("GitHub Copilot", &known));
        assert!(is_ai_name("Helper-Bot", &known));
        assert!(!is_ai_name("Jane Doe", &known));
    }
}
