# Agent instructions

Guidance for AI coding agents working in this repository. Read
[README.md](README.md) for what the site shows and how the configuration works, and
[DEVELOPMENT.md](DEVELOPMENT.md) for the architecture, the data collection, and the
build.

## GitHub text: never tag people

Never write `@` followed by a GitHub username in pull request titles or descriptions,
issue or review comments, commit messages, or any text a workflow posts to GitHub,
such as the company discovery report. GitHub notifies every account mentioned this
way, and these PRs often name many real contributors.

- Link an account without the `@`: `[octocat](https://github.com/octocat)`.
- Put free-form text that may contain handles, such as profile company values or
  organization aliases like `@unionai`, in code spans, where GitHub creates no
  mention.
- The only intended mention is the discovery workflow's `@Copilot` review request.

## Commits

- Sign off every commit with `git commit -s`; the DCO check requires it.
- Credit AI help with an `Assisted-By` trailer naming the tool and model, for example
  `Assisted-By: Claude Opus 5.5 <noreply@anthropic.com>`. Do not use
  `Co-Authored-By` for AI help.
- Keep messages short: a concise subject and, at most, a brief body.

## Pull requests

- Write a short plain summary of what changed and why, without headings or
  generated-by footers. There is no PR template.
- Run the checks CI runs before opening one:

  ```sh
  cargo fmt --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  npm ci && npm run lint && npm run format:check
  ```

## Code and data

- Do not add new places that store email addresses. The collector keeps commit
  author and committer addresses only as truncated hashes and domains.
- `config/companies.yaml` and `config/affiliations.yaml` must stay sorted; run
  `cargo run -p pulse -- format-config`. Put the evidence for an affiliation in the
  pull request.
- A change to the snapshot format needs a new `MAGIC` version in
  `crates/pulse-core/src/snapshot.rs`; the site and its data then deploy together.
- The web views share the components in `web/js/components` (tables, charts,
  filters); use them rather than writing new markup for the same thing.
- `scripts/build-site.sh` ends with a smoke test that starts the built WebAssembly
  module; keep it passing.
- Keep README.md for people who use the site or maintain its configuration, and put
  technical detail in DEVELOPMENT.md.
