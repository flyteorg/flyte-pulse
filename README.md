# Flyte Pulse data

Collected state for [Flyte Pulse](https://github.com/flyteorg/flyte-pulse), updated
by the daily `Sync and deploy` workflow. Each sync is one commit, so the history of
the data is kept here.

`state/` holds sorted JSON lines: `manifest.json` (schema version, generation time,
and per-repository sync cursors), `repositories.jsonl`, `contributors.jsonl`,
`email_identities.jsonl` (truncated SHA-256 email hashes, never raw emails), and
`pull_requests/`, `commits/`, `pull_request_commits/` split per repository.

The latest state is also published as `pulse-state.json.gz` on the
[`pulse-state` release](https://github.com/flyteorg/flyte-pulse/releases/tag/pulse-state).
Convert between the formats with `pulse convert --from state --to pulse-state.json.gz`.
