# Flyte Pulse (WebAssembly)

Community activity across the Flyte repositories, served as a static GitHub Pages
site. Rust collects the data in GitHub Actions and answers every dashboard query
in the browser through WebAssembly; there is no database or API server.

Site: https://flyte.org/flyte-pulse/

```
GitHub Actions (daily)                          GitHub Pages
┌───────────────────────────────┐   pulse.bin.gz   ┌──────────────────────────────┐
│ pulse sync                    │ ───────────────▶ │ index.html + JS views        │
│   Git clones → commits, links │                  │ pulse_web.wasm (Rust)        │
│   GitHub GraphQL → PRs, people│                  │   loads the snapshot, answers│
│ pulse build → snapshot        │                  │   /api/... queries in-page   │
└──────────────┬────────────────┘                  └──────────────────────────────┘
               │ state
               ├─▶ data branch: state/ (JSON lines, full history)
               └─▶ release "pulse-state": pulse-state.json.gz (latest copy)
```

## Layout

| Path | What it is |
|---|---|
| `crates/pulse-core` | Data model, snapshot format, and every dashboard query. Compiles natively and to WebAssembly. |
| `crates/pulse` | `pulse` CLI: `sync` (collector), `build` (snapshot), `convert` (state formats). |
| `crates/pulse-web` | WebAssembly wrapper (`Pulse.load`, `pulse.request(url, today)`). |
| `tools/pulse-seed` | One-time import of an existing raw Parquet dataset. Not built by default. |
| `web/` | Single-page site: `index.html`, hash-routed JS views, CSS. |
| `config/` | Reviewed catalogs: `repositories.yaml`, `companies.yaml`, `affiliations.yaml`. |
| `scripts/build-site.sh` | Builds the site into `dist/`: WebAssembly, web assets, and the data snapshot. |

## How the data is collected

`pulse sync` updates the state for every `ACTIVE` repository in
`config/repositories.yaml`:

- **Commits come from Git, not the API.** The first sync of a repository (no Git
  cursor yet, a reseed, or changed target branches) makes a full bare clone of the
  target branches plus GitHub's `refs/pull/*/head` refs. Later syncs fetch with
  `--shallow-since` from the last cursor (minus `--overlap-days`), plus the refs of
  pull requests whose head changed. Commit details, line stats, and branch
  membership are read from the clone, which is deleted afterwards.
- **PR→commit links come from Git too:** `git rev-list <PR head> --not <target
  branches>` (or `<head> --not <merge>^1` for merge commits). This matches GitHub's
  PR commit lists without one API request per pull request. When a range reaches
  the shallow boundary, the fetch is deepened and the range recomputed.
- **The GraphQL API is used only for what Git cannot provide:** pull request
  metadata (about one point per 100 PRs), batched lookups of commit emails without
  a known account, and a 90-day public profile refresh. Emails are never stored;
  the state keeps a truncated SHA-256 per email.
- Identifiers are stable across syncs: repositories are `github.com:<node id>`,
  people are `github:<account id>`, and pull requests use GitHub's numeric IDs.

`pulse build` precomputes daily metrics, company credit, and AI attribution
(Co-Authored-By / Assisted-By trailers, AI identities), and writes `pulse.bin.gz`
(postcard, gzip) plus `meta.json`. The browser downloads the snapshot once; each
page then asks the WebAssembly module for an HTTP-style URL such as
`/api/dashboard?days=30` and renders the JSON it returns.

Counting rules:

- Commits count when they are listed on a tracked pull request. Git also returns
  direct pushes and imported history; they stay in the state but are not counted.
- Bot activity (GitHub bot accounts, `[bot]` logins, and `identity_rules` in
  `repositories.yaml`, which classify `web-flow` as a bot) is kept but not counted.
- Each merged PR's full line diff is shared by its author and recognized human or
  AI co-authors; company credit uses the affiliation on each event's date.
- The site is public and has no login. It shows public GitHub activity and the
  reviewed company mappings from `config/`.

## State

The collected state (repositories, contributors, pull requests, commits, links,
watermarks) has two interchangeable formats; `pulse` picks one from the path:

- `*.json.gz` / `*.json`: one JSON document. Published as the `pulse-state.json.gz`
  asset of the `pulse-state` release after every sync.
- any other path: a directory of sorted JSON-lines files (`manifest.json`,
  `repositories.jsonl`, `contributors.jsonl`, `email_identities.jsonl`, and
  `pull_requests/`, `commits/`, `pull_request_commits/` split per repository).
  Committed to `state/` on the orphan `data` branch, so the history of every
  sync is kept with small diffs.

```sh
pulse convert --from pulse-state.json.gz --to state   # file → directory
pulse convert --from state --to pulse-state.json.gz   # directory → file
```

## Commands

```sh
cargo build --release -p pulse

# Collect (token: PULSE_GITHUB_TOKEN, else GITHUB_TOKEN; read-only access to public repos)
GITHUB_TOKEN=$(gh auth token) ./target/release/pulse sync --state state
#   --only owner/name     restrict to repositories (repeatable)
#   --reseed owner/name   discard a repository's data and collect it again
#   --history-days N      PR history for repositories without a cursor (0 = all)

# Build the site into dist/ and serve it
scripts/build-site.sh state dist
python3 -m http.server -d dist 8000   # http://localhost:8000
```

`rust-toolchain.toml` is the only place the Rust version is set; rustup installs
that toolchain with the `wasm32-unknown-unknown` target on first use.
`scripts/build-site.sh` also needs `wasm-bindgen-cli` matching the `wasm-bindgen`
version in `Cargo.lock` (for example `cargo install wasm-bindgen-cli --version
0.2.129 --locked`); `wasm-opt` (binaryen) is used when present.

The views are plain ES modules with no runtime dependencies. ESLint and Prettier
check them: `npm ci`, then `npm run lint` and `npm run format:check`
(`npm run format` fixes formatting).

### Seeding (one time)

The initial state was imported from an existing raw dataset (a directory with
`manifest.json` and Parquet tables) instead of scanning all history through the API:

```sh
cargo run --release -p pulse-seed -- --raw-dir /path/to/raw-dataset --out seed.json.gz
GITHUB_TOKEN=$(gh auth token) ./target/release/pulse sync --state seed.json.gz --out state
```

The dataset's cursors become PR scan cursors, so the first sync scans pull
requests incrementally while making a full Git clone of every repository.

## GitHub setup

1. Push `main` and the `data` branch.
2. Settings → Pages → Source: **GitHub Actions**.
3. Optional: add a `PULSE_GITHUB_TOKEN` secret (a fine-grained token with public
   read access) for a higher rate limit; the workflow's `GITHUB_TOKEN` is enough
   for daily syncs.

Workflows:

- `sync.yml` runs daily at 02:17 UTC: sync, commit the state to `data`, replace
  the release asset, build, and deploy Pages. Each run's summary page reports what
  the sync collected (per repository, with API usage) and the published site data
  (`pulse sync --summary-file`, `pulse summary`). Run it manually to reseed
  repositories (`reseed: owner/name ...`) or to deploy without syncing. Pushes to
  `main` that touch code, web, or config rebuild and deploy from the saved state.
- `ci.yml` runs formatting, clippy, tests, the WebAssembly build, ESLint, and
  Prettier with a read-only token.

Actions are pinned by commit SHA; jobs get write access only where they need it.
Dependabot opens one grouped pull request per week for each of Rust, JS, and
GitHub Actions, proposing only releases that are at least 7 days old.

To add a repository, add it to `config/repositories.yaml`; the next sync makes a
full clone and collects all of its pull requests. Company and affiliation changes
take effect on the next build; no resync is needed. Keep `companies.yaml` sorted by
company ID and `affiliations.yaml` by username with `pulse format-config` (tests
fail otherwise); comments inside an entry move with it.

## Company and affiliation discovery

After each sync, `pulse discover --apply` proposes updates to `companies.yaml` and
`affiliations.yaml`, and `scripts/discovery-pr.sh` opens or updates the
`bot/company-discovery` pull request with the evidence. Merging it approves the
changes. The signals are:

- public profile company values, refreshed weekly and kept as first-seen and
  last-seen runs, so a change is dated within a week of happening;
- public organization memberships, matched through `@org` company aliases;
- corporate commit email domains read from Git (free-mail, noreply, placeholder,
  and alumni domains are dropped; only the domain is stored, never the address),
  matched through a company's `domains` or its website.

From these it proposes email domains and organization aliases for known companies,
new companies (from a single `@org` named in a profile, or an organization whose
website is an email domain several people use), affiliations for contributors
without one, affiliation changes (the old period ends when the new value was first
seen), and earlier affiliations from a clean switch of email domains. The current
affiliation then starts where the earlier one ends, including an existing undated
entry. Entries are inserted in sorted order. Anything
ambiguous, such as several companies in one profile or a conflict with a curated
mapping, is listed for a person instead. Values in `config/discovery.yaml` are
never proposed again.

The `main` ruleset requests a Copilot review on every push, and the workflow posts
an `@Copilot` comment asking it to verify company names and websites, that each
account is a real person, and that the evidence supports each affiliation. Copilot
acts on mentions from users with Copilot access, so set a `PULSE_COPILOT_TOKEN`
secret (a user token) for that. Pull requests opened with the workflow token do not
trigger CI, so the script dispatches `ci.yml` on the branch for the required checks.
Once someone else pushes to the branch, the workflow stops updating it until it is
merged or closed. Run the workflow with `full_git` once to backfill email domains for
history collected before this feature.

New contributors whose login, name, or commit email contains "bot" are classified
as bots, unless they have a curated affiliation; existing accounts like that are
listed for review. `identity_rules` always win.

## License

Apache License 2.0, like the other Flyte repositories. See [LICENSE](LICENSE).
