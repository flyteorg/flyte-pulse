// Columns, tables, and charts for the records several pages show: people, repositories,
// pull requests, commits, and AI models.

import {
  aiContributorLink,
  aiIdentityLinks,
  escapeHtml,
  externalLink,
  number,
  repositoryLink,
  userLink,
} from "../common.js";
import { sharePie } from "./charts.js";
import { column, dataTable } from "./table.js";

const self = (row) => row;

// `person(row)` returns { id, name, login }; `options` extend the column.
export function personColumn(label = "Contributor", person = self, options = {}) {
  return column.text("person", label, {
    value: (row) => {
      const p = person(row);
      return (p.name || p.login || "").toLocaleLowerCase();
    },
    cell: (row) => {
      const p = person(row);
      return userLink(p.id, p.name, p.login);
    },
    filterValue: (row) => {
      const p = person(row);
      return [p.name || "", p.login || ""];
    },
    ...options,
  });
}

// `repository(row)` returns { id, name, target_branch }; `options` extend the column.
export function repositoryColumn(label = "Repository", repository = self, options = {}) {
  return column.text("repository", label, {
    value: (row) => repository(row).name || "",
    cell: (row) => {
      const r = repository(row);
      return repositoryLink(r.id, r.name, r.target_branch);
    },
    ...options,
  });
}

// A filter on the tracked branches of repository rows: `branches` lists the branches a row
// covers (one for a branch row, all of them for a repository-wide row).
export function targetBranchFilter() {
  return {
    key: "target_branch",
    label: "Target branch",
    value: (row) => row.branches || row.target_branch,
  };
}

// Dashboard contributor rows (overview lists, repository top contributors).
export function contributorColumns({ first = false } = {}) {
  return [
    personColumn(),
    ...(first ? [column.date("first_contribution_date", "First recorded")] : []),
    column.count("pull_requests_merged", "Merged", { title: "Merged pull requests" }),
    column.count("commits", "Commits"),
    column.count("pull_requests_opened", "Opened", { title: "Opened pull requests" }),
    column.count("shared_additions", "Added", { title: "Lines added, shared with co-authors" }),
    column.count("shared_deletions", "Removed", {
      title: "Lines removed, shared with co-authors",
    }),
  ];
}

export function reviewerColumns() {
  return [
    personColumn("Reviewer"),
    column.count("reviews", "Reviewed", { title: "Pull requests reviewed" }),
    column.count("pull_requests_merged", "Merged", { title: "Merged pull requests" }),
    column.count("commits", "Commits"),
  ];
}

function providerColumn(key, options = {}) {
  return column.text("provider", "Provider", {
    cell: (row) => aiContributorLink(row[key], row.provider, "", row.provider, row.provider),
    ...options,
  });
}

// AI model rows (overview AI list, AI activity page).
export function aiModelColumns() {
  return [
    providerColumn("provider_key", { filter: "choice", filterValue: (row) => row.provider }),
    column.text("model", "Model"),
    column.count("commits_with_ai", "Commits with AI"),
    column.count("commits_ai_authored", "AI-authored commits"),
    column.count("commits_ai_coauthored", "AI co-authored commits"),
    column.count("commits_ai_assisted", "AI-assisted commits"),
    column.count("pull_requests_with_ai", "PRs with AI"),
    column.count("shared_additions", "Lines added · shared"),
    column.count("shared_deletions", "Lines removed · shared"),
  ];
}

export function aiProviderColumns() {
  return [
    providerColumn("key", { filter: "choice", filterValue: (row) => row.provider }),
    column.count("commits_with_ai", "Commits with AI"),
    column.count("commits_ai_authored", "AI-authored commits"),
    column.count("commits_ai_coauthored", "AI co-authored commits"),
    column.count("commits_ai_assisted", "AI-assisted commits"),
    column.count("pull_requests_with_ai", "PRs with AI"),
    column.count("shared_additions", "Lines added · shared"),
    column.count("shared_deletions", "Lines removed · shared"),
  ];
}

const categoryNames = (row) =>
  (row.categories || []).map((category) => category.replaceAll("_", " "));
const categoryText = (row) => categoryNames(row).join(", ");

// "open · stale" for an open pull request without activity for 60 days, else its state.
const stateLabel = (row) => (row.stale ? "open · stale" : row.state);

// Days from opening to the merge, or to today while open; unknown for closed ones.
const DAY_MS = 86400000;
function daysOpen(row) {
  const end = row.merged_at || (row.state === "open" ? Date.now() : null);
  if (!row.created_at || !end) return null;
  return Math.max(0, Math.floor((new Date(end) - new Date(row.created_at)) / DAY_MS));
}

// Pull requests, newest first unless `sort` says otherwise. `repository`, `author`, and
// `state` add those columns; `attribution` adds the AI attribution categories; `activity`
// adds the last activity and the days open.
export function pullRequestTable(id, rows, options = {}) {
  const {
    repository = true,
    author = true,
    state = true,
    attribution = false,
    activity = false,
  } = options;
  const columns = [
    column.date("created_at", "Created"),
    column.text("title", "Pull request", {
      value: (row) => row.number,
      cell: (row) => externalLink(row.url, `#${row.number} ${row.title}`),
      className: "table-link",
    }),
    ...(repository
      ? [
          repositoryColumn(
            "Repository",
            (row) => ({
              id: row.repository_id,
              name: row.repository,
              target_branch: row.target_branch,
            }),
            { filter: "choice" },
          ),
        ]
      : []),
    column.text("target_branch", "Target branch", { filter: "choice" }),
    ...(author
      ? [
          personColumn(
            "Author",
            (row) => ({ id: row.author_id, name: row.author_name, login: row.author_login }),
            { filter: "text" },
          ),
        ]
      : []),
    ...(activity
      ? [
          column.date("updated_at", "Last activity", {
            title: "Last push, comment, review, or label change",
          }),
          column.count("days_open", "Days open", {
            title: "Days from opening to the merge, or to today while open",
            value: daysOpen,
            cell: (row) => {
              const days = daysOpen(row);
              return days == null ? "—" : number.format(days);
            },
          }),
        ]
      : []),
    ...(state
      ? [
          column.text("state", "State", {
            value: stateLabel,
            title: "Open pull requests without activity for 60 days are stale",
            cell: (row) =>
              row.stale
                ? `${escapeHtml(row.state)} · <span class="stale-label">stale</span>`
                : escapeHtml(row.state || "—"),
            filter: "choice",
            // A stale pull request matches both "open" and "stale".
            filterValue: (row) => (row.stale ? [row.state, "stale"] : row.state),
          }),
        ]
      : []),
    ...(attribution
      ? [
          column.text("categories", "AI attribution", {
            value: (row) => categoryText(row) || "AI attribution",
            filter: "choice",
            filterValue: categoryNames,
          }),
        ]
      : []),
  ];
  return dataTable({
    id,
    columns,
    rows,
    empty: options.empty || "No pull requests to show.",
    sort: options.sort,
  });
}

function attribution(row) {
  const lines = [
    row.author_name && `Author: ${userLink(row.author_id, row.author_name, row.author_login)}`,
    row.committer_name &&
      `Committer: ${userLink(row.committer_id, row.committer_name, row.committer_login)}`,
    row.ai_coauthors?.length && `Co-Authored-By: ${aiIdentityLinks(row.ai_coauthors)}`,
    row.ai_assistants?.length && `Assisted-By: ${aiIdentityLinks(row.ai_assistants)}`,
  ].filter(Boolean);
  return `${escapeHtml(attributionText(row))}${lines.map((line) => `<small>${line}</small>`).join("")}`;
}

const attributionText = (row) => (row.categories || []).join(", ") || row.role || "Author";

export function commitTable(id, rows, options = {}) {
  return dataTable({
    id,
    columns: [
      column.date("committed_at", "Committed"),
      column.text("subject", "Commit", {
        value: (row) => row.subject || row.message || row.sha,
        cell: (row) => externalLink(row.url, row.subject || row.message || row.sha),
        className: "table-link",
      }),
      repositoryColumn("Repository", (row) => ({ id: row.repository_id, name: row.repository }), {
        filter: "choice",
      }),
      column.text("target_branches", "Target branches", {
        value: (row) => (row.target_branches || []).join(" "),
        cell: (row) =>
          (row.target_branches || [])
            .map((branch) => repositoryLink(row.repository_id, branch, branch))
            .join(" ") || "—",
        filter: "choice",
        filterValue: (row) => row.target_branches || [],
      }),
      column.text("attribution", "Attribution", {
        value: attributionText,
        cell: attribution,
        className: "table-link",
        filter: "choice",
        filterValue: (row) => (row.categories?.length ? row.categories : [row.role || "Author"]),
      }),
    ],
    rows,
    empty: options.empty || "No commits to show.",
  });
}

export const METRICS = {
  merged: { key: "pull_requests_merged", label: "Merged PRs", unit: "merged PRs" },
  commits: { key: "commits", label: "Commits", unit: "commits" },
  opened: { key: "pull_requests_opened", label: "Opened PRs", unit: "opened PRs" },
  reviews: { key: "reviews", label: "Reviews", unit: "reviews" },
};

// Activity split by repository. `metrics` are METRICS entries, optionally with a subtitle.
export function repositoryShare(id, rows, metrics, options = {}) {
  return sharePie({
    id,
    title: "Repository share",
    rows,
    metrics,
    link: (row) => repositoryLink(row.id, row.name, row.target_branch),
    ...options,
  });
}
