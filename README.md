# Flyte Pulse

Community activity across the Flyte repositories: who contributes, from which
companies, how code review is going, and how AI tools are used. It is rebuilt every
day from public GitHub activity.

**Site:** https://flyteorg.github.io/flyte-pulse/

## What the site shows

- **Overview**: a leaderboard (top contributor, top community contributor, top new
  contributor, top reviewer, most used AI model), totals for the selected time range
  and repository, activity over time, ranked contributor lists, repository and company
  shares, code review coverage, and the languages in use.
- **Repositories**: every tracked repository and branch, with its activity, top
  contributors and reviewers, companies, and recent pull requests.
- **Contributors**: new and active contributors. Each person's page shows their
  activity, company affiliations, reviews, AI usage, pull requests, and commits.
- **Journey**: the second tab of a contributor's page: a timeline from their first
  contribution to their latest activity (first merged pull request, 100 merged pull
  requests, first contribution to each repository, anniversaries, affiliation changes)
  and achievements with the date each was unlocked. Each achievement has a button that
  copies a link to the page with that badge highlighted.
- **Companies**: contributions credited to each company and the people affiliated
  with it.
- **AI Activity**: AI authorship and assistance found in commit trailers and AI
  accounts.

Every table sorts by any column, and tables of pull requests, commits, repositories
and AI models can be filtered by repository, target branch, state, author or
provider. Filters and sort order are kept in the page address, so any view can be
shared as a link.

## How activity is counted

- Only public GitHub activity is used, refreshed daily at 02:17 UTC. Commit author
  and committer email addresses are stored only as truncated hashes and domains.
  Commit messages are kept as written, so addresses in trailers such as
  `Signed-off-by` remain in the collected data; the site shows only each message's
  first line.
- A commit counts when it is part of a pull request to a tracked branch. Direct
  pushes and imported history are kept but not counted.
- Bots are not counted: GitHub bot accounts, `[bot]` logins, and accounts listed as
  bots in `config/repositories.yaml`.
- A merged pull request's lines are shared by its author and its co-authors
  (`Co-Authored-By`), so per-person line totals overlap.
- A reviewer counts once per pull request, on the day of their first review.
  Reviews of one's own pull requests and reviews by bots and AI accounts do not
  count, and reviewing alone does not make someone an active or new contributor.
  Review coverage is the share of merged pull requests that someone other than the
  author reviewed before the merge; time to first review runs from opening,
  including time as a draft.
- A new contributor made their first recorded contribution in the selected range.
- Each contribution is credited to the company the person was affiliated with on
  that day, as recorded in `config/affiliations.yaml`. Without an affiliation it
  counts as Independent.
- Community activity is activity by people while they are not affiliated with a
  company that maintains Flyte (Union).
- AI attribution comes from AI accounts and from `Co-Authored-By` and `Assisted-By`
  commit trailers that name a recognized AI tool. Point releases of a model count as
  one model (Opus 4.5 and Opus 4.6 are both "Opus 4"); the provider page lists the
  exact versions seen.

## Common tasks

Everything below is a pull request that changes a file in `config/`. When it merges,
the site rebuilds within minutes. Company and affiliation changes apply to all past
activity; nothing needs to be collected again.

### Track a new repository

Add an entry to the end of `repositories:` in `config/repositories.yaml`:

```yaml
  - provider: GITHUB
    host: github.com
    owner: flyteorg
    name: my-repo
    module: sdk          # optional grouping label
    status: ACTIVE
    target_branches:     # branches whose pull requests count
      - name: main
        label: v2        # optional display name when several branches are tracked
```

The repository must be public. The next daily sync collects its full history, which
includes all of its pull requests. To collect it sooner, run **Actions → Sync and
deploy → Run workflow**. To stop collecting a repository but keep its data, set
`status: INACTIVE`.

### Add or fix a company affiliation

The easiest way is for the person to set the **Company** field of their GitHub
profile to the company's name or GitHub organization (such as `@unionai`). The
discovery bot notices within about a week and proposes the affiliation in its pull
request (see [below](#review-the-company-discovery-pull-request)).

To set it directly, edit `config/affiliations.yaml`:

```yaml
- username: octocat
  companies:
  - company: union                # no dates: every day the periods below don't cover
  - company: lyft                 # a period: from is included, until is not
    from: '2019-01-01'
    until: '2022-03-01'
  - company: independent          # no company from this day on
    from: '2024-06-01'
```

- One entry per person, listing every company their contributions are credited to.
  A company without `from` covers every day that no dated period covers; a person
  can have one such company.
- `from` is included and `until` is not (UTC days). Periods cannot overlap, and
  `until` needs a `from`. The dates bound the credited contributions; they are not
  a claim about employment dates.
- `company` must be an `id` in `config/companies.yaml` (see the next section), or
  `independent`.
- Usernames ignore case. If a username is ambiguous or was renamed, use
  `contributor_id: github:<numeric account ID>` instead.
- Keep the file sorted by username (ignoring case), and each person's companies by
  start date; CI checks the order. With Rust installed,
  `cargo run -p pulse -- format-config` sorts it.
- Put the evidence in the pull request description, such as a profile, a company
  email domain in commits, or a public announcement.
- If the person's GitHub profile names a different company, also add their username
  under `ignore.users` in `config/discovery.yaml`, so the bot stops flagging the
  difference.

### Add a company

Add an entry to `config/companies.yaml`, sorted by `id`:

```yaml
- id: example-inc                   # lowercase letters, digits, '-' or '_'
  canonical_name: Example Inc.
  website_url: https://example.com  # must be HTTPS
  aliases:                          # GitHub profile Company values that mean this company
  - '@example-org'                  # a GitHub organization, quoted because of the @
  - Example, Inc.
  domains:                          # optional: corporate commit email domains
  - example.com
```

The canonical name also counts as an alias. An alias or a domain can belong to only
one company. Use `aliases: []` when there are none. Set `maintainer: true` only for a
company that maintains Flyte: its people's activity is not counted as community
activity.

### Mark an account as a bot, an AI, or a person

Accounts are classified from GitHub's account type and their names. To override the
classification, add a rule under `identity_rules` in `config/repositories.yaml`:

```yaml
identity_rules:
  - provider: GITHUB
    login: example-release-bot
    kind: BOT            # HUMAN, BOT, AI, or UNKNOWN
```

### Review the company discovery pull request

After each daily sync, the discovery bot opens or updates the **Company and
affiliation updates** pull request (branch `bot/company-discovery`). It proposes new
companies, aliases, and email domains, affiliations for people without one, and
dated changes when someone moves. Each change lists its evidence: a profile company
value, a public organization membership, or a corporate commit email domain.
Anything ambiguous is listed under "Needs a person" and left unchanged.

To review it:

1. Check the evidence for each change. Fix anything wrong by editing the branch.
   After someone else pushes, the bot stops updating the pull request until it is
   merged or closed.
2. Approve the held CI run ("Approve workflows to run"), which GitHub requires for
   pushes made by workflows.
3. Merge. Merging approves the changes.

To stop the bot from proposing something again, add it to `config/discovery.yaml`:

| List | For |
|---|---|
| `profile_companies` | profile Company values that do not name an employer, such as "Student" |
| `organizations` | GitHub organizations that say nothing about an employer |
| `domains` | email domains that do not identify an employer |
| `users` | people whose affiliations are maintained by hand only |

## Questions and development

Open an issue or a pull request in this repository. To build the site locally or
work on the code, see [DEVELOPMENT.md](DEVELOPMENT.md).

## License

Apache License 2.0, like the other Flyte repositories. See [LICENSE](LICENSE).
