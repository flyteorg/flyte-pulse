//! Shared Flyte Pulse model: collected state, the browser snapshot, and dashboard queries.

pub mod snapshot;
pub mod state;
pub mod time;
pub mod views;

/// Common AI tool names recognized in commit trailers.
pub const AI_NAME_ALIASES: [&str; 16] = [
    "amazon q",
    "anthropic",
    "chatgpt",
    "claude",
    "claude code",
    "codex",
    "copilot",
    "cursor",
    "devin",
    "gemini",
    "github copilot",
    "openai",
    "openai codex",
    "q developer",
    "replit ai",
    "windsurf",
];
