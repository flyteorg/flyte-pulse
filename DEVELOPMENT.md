# Developing Flyte Pulse

How the site is built, how the data is collected, and how to work on the code. For
what the site shows and how to change the tracked repositories or company
affiliations, see the [README](README.md).

Rust collects the data in GitHub Actions and answers every dashboard query in the
browser through WebAssembly; there is no database or API server.

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

## Running locally

`scripts/serve-local.sh` fetches the latest published state from the `data` branch,
builds the site into `dist/`, and serves it at http://127.0.0.1:8000:

```sh
scripts/serve-local.sh          # or: scripts/serve-local.sh 9000
STATE=path scripts/serve-local.sh   # build from a local state file or directory instead
```

It needs Rust (`rust-toolchain.toml` sets the version; rustup installs it with the
`wasm32-unknown-unknown` target on first use), `wasm-bindgen-cli` matching the
`wasm-bindgen` version in `Cargo.lock` (for example `cargo install wasm-bindgen-cli
--version 0.2.129 --locked`), and Python 3. `wasm-opt` (binaryen) is used when
present. Use a recent binaryen (CI pins release 133): older ones, such as Ubuntu's
108, produce a module that fails to start. Each build ends with a Node smoke test
(`scripts/smoke-site.mjs`) that starts the module, loads the snapshot, and answers a
query.

Before opening a pull request, run the same checks as CI:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm ci && npm run lint && npm run format:check   # npm run format fixes formatting
```

The views are plain ES modules with no runtime dependencies; ESLint checks the
JavaScript and Prettier the JavaScript, HTML, and CSS.

Commit and pull request conventions, including the DCO sign-off, the `Assisted-By`
trailer for AI help, and never tagging people with `@` in GitHub text, are in
[AGENTS.md](AGENTS.md). They apply to people and AI agents alike.

## Layout

| Path | What it is |
|---|---|
| `crates/pulse-core` | Data model, snapshot format, and every dashboard query. Compiles natively and to WebAssembly. |
| `crates/pulse` | `pulse` CLI: `sync` (collector), `build` (snapshot), `convert` (state formats). |
| `crates/pulse-web` | WebAssembly wrapper (`Pulse.load`, `pulse.request(url, today)`). |
| `web/` | Single-page site: `index.html`, hash-routed JS views, CSS. Pages share the components in `web/js/components` (sortable tables, charts, filters). |
| `config/` | Reviewed catalogs: `repositories.yaml`, `companies.yaml`, `affiliations.yaml`, and the discovery settings in `discovery.yaml`. |
| `scripts/build-site.sh` | Builds the site into `dist/`: WebAssembly, web assets, and the data snapshot, then runs the smoke test. |
| `scripts/serve-local.sh` | Fetches the data branch's state, builds the site, and serves it locally. |
| `scripts/smoke-site.mjs` | Starts a built site's module in Node and answers a query. |
| `scripts/discovery-pr.sh` | Opens or updates the company discovery pull request. |

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
  metadata with each PR's submitted reviews (about one point per 100 PRs; the first
  50 reviews come with the PR, and a longer review list costs one more request per
  100 reviews), batched lookups of commit emails without a known account, a 90-day
  public profile refresh, and each repository's language breakdown (one point per
  repository per sync). Commit author and committer emails are kept only as a
  truncated SHA-256 and the domain; commit messages are stored as written, including
  trailer addresses such as `Signed-off-by`.
- When the collector starts reading a new pull request field (reviews were the
  first), the next sync reads every pull request of each repository once more and
  records that in the repository's watermark (`pr_fields_version`). It needs no new
  clone: Git work stays what an incremental sync would do, so older pull requests
  keep their saved commit links. For all tracked repositories this costs about 200
  points; pages of old pull requests get smaller when GitHub starts timing out.
- Identifiers are stable across syncs: repositories are `github.com:<node id>`,
  people are `github:<account id>`, and pull requests use GitHub's numeric IDs.

`pulse build` precomputes daily metrics, company credit, and AI attribution
(Co-Authored-By / Assisted-By trailers, AI identities), and writes `pulse.bin.gz`
(postcard, gzip) plus `meta.json`. The browser downloads the snapshot once; each
page then asks the WebAssembly module for an HTTP-style URL such as
`/api/dashboard?days=30` and renders the JSON it returns.

The counting rules are described for readers in the README's
[How activity is counted](README.md#how-activity-is-counted) section.

## State

The collected state (repositories, contributors, pull requests and their reviews,
commits, links, watermarks) has two interchangeable formats; `pulse` picks one from the path:

- `*.json.gz` / `*.json`: one JSON document. Published as the `pulse-state.json.gz`
  asset of the `pulse-state` release after every sync.
- any other path: a directory of sorted JSON-lines files (`manifest.json`,
  `repositories.jsonl`, `contributors.jsonl`, `email_identities.jsonl`, and
  `pull_requests/`, `pull_request_reviews/`, `commits/`, `pull_request_commits/`
  split per repository).
  Committed to `state/` on the orphan `data` branch, so the history of every
  sync is kept with small diffs.

```sh
pulse convert --from pulse-state.json.gz --to state   # file → directory
pulse convert --from state --to pulse-state.json.gz   # directory → file
```

## Collecting data

```sh
cargo build --release -p pulse
# Token: PULSE_GITHUB_TOKEN, else GITHUB_TOKEN; read-only access to public repositories.
GITHUB_TOKEN=$(gh auth token) ./target/release/pulse sync --state state
#   --only owner/name     restrict to repositories (repeatable)
#   --reseed owner/name   discard a repository's data and collect it again
#   --history-days N      PR history for repositories without a cursor (0 = all)

# Build the site into dist/ from that state and serve it
scripts/build-site.sh state dist
python3 -m http.server -d dist 8000
```

### Starting from nothing

The workflow refuses to run without a saved state, so the first state is created
locally: `pulse sync` with a missing `--state` path starts empty and collects every
active repository's full history (a full clone plus all of its pull requests). Push
the resulting `state/` directory to the orphan `data` branch; from then on the daily
sync is incremental.

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

Configuration rules checked by the tests: `companies.yaml` is sorted by company ID
and `affiliations.yaml` by username (ignoring case), then start date. Run `pulse
format-config` to sort both files; comments inside an entry move with it. Company and
affiliation changes take effect on the next build; adding a repository takes effect on
the next sync, which makes a full clone and collects all of its pull requests.

## Company and affiliation discovery internals

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
secret (a user token) for that; without it the comment is posted once, when the pull
request is opened. GitHub holds CI for pushes by the workflow, so approve the CI run
on the pull request ("Approve workflows to run") after each update to get the
required checks.
Once someone else pushes to the branch, the workflow stops updating it until it is
merged or closed. Run the workflow with `full_git` once to backfill email domains for
history collected before this feature.

New contributors with a login or commit email word ending in "bot" (`snyk-bot`,
`goreleaserbot`) or a display name with the word "bot" or "robot" are classified as
bots, unless they have a curated affiliation; existing accounts like that are listed
for review. `identity_rules` always win.
