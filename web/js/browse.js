import { getJson } from "./api.js";
import {
  aiContributorLink,
  content,
  encodePath,
  escapeHtml,
  externalLink,
  number,
  page,
  pageHead,
  panel,
  pendingPanel,
  rangeStamp,
  reviewPanel,
  stats,
} from "./common.js";
import {
  bindCompanyFilters,
  companyDetailPage,
  companyList,
  companyProfile,
  loadCompanyShare,
} from "./companies.js";
import { activityPanel, languagePanel, sharePie } from "./components/charts.js";
import {
  bindFilters,
  filterBar,
  rangeField,
  rangeParam,
  searchField,
  selectField,
  setParam,
  textMatches,
} from "./components/controls.js";
import {
  METRICS,
  aiModelColumns,
  aiProviderColumns,
  commitTable,
  contributorColumns,
  personColumn,
  pullRequestTable,
  repositoryColumn,
  repositoryShare,
  reviewerColumns,
} from "./components/entities.js";
import { column, dataTable } from "./components/table.js";
import { currentRoute, replaceParams } from "./route.js";

let currentContributorData = null;

function repositoryList(data) {
  const rows = data.repositories.map((repository) => ({ ...repository, ...repository.summary }));
  return (
    pageHead(
      "Repositories",
      "Activity across tracked repositories and target branches, based on the last 365 days.",
    ) +
    panel({
      title: "Tracked repositories",
      body: dataTable({
        id: "repositories",
        columns: [
          repositoryColumn(),
          column.text("status", "Status"),
          column.count("pull_requests_merged", "Merged PRs"),
          column.count("commits", "Commits"),
          column.count("active_contributors", "Contributors"),
          column.percent("review_coverage", "Review coverage"),
        ],
        rows,
        empty: "No repositories are in the catalog yet.",
        sort: { key: "pull_requests_merged", direction: "desc" },
      }),
    })
  );
}

// Contributors whose first (`view` "new") or last recorded contribution is in the range.
function contributorTable(users, query, view, days, repositoryId) {
  const today = new Date().toISOString().slice(0, 10);
  const start =
    days === "all"
      ? ""
      : new Date(Date.now() - (Number(days) - 1) * 86400000).toISOString().slice(0, 10);
  const firstDate = (item) =>
    repositoryId
      ? item.first_contribution_by_repository?.[repositoryId]
      : item.first_contribution_date;
  const lastDate = (item) =>
    repositoryId
      ? item.last_contribution_by_repository?.[repositoryId]
      : item.last_contribution_date;
  const rows = users.filter((item) => {
    if (!textMatches(query, [item.login || "", item.name || ""])) return false;
    const date = view === "new" ? firstDate(item) : lastDate(item);
    if (!date) return days === "all" && !repositoryId && view !== "new";
    return date >= start && date <= today;
  });
  const empty = query.trim()
    ? `No contributors match “${query.trim()}”.`
    : view === "new"
      ? "No new contributors in this range."
      : "No contributor activity in this range.";
  const table = dataTable({
    id: view === "new" ? "new-contributors" : "all-contributors",
    columns: [
      personColumn(),
      column.date("first", "First recorded", { value: firstDate }),
      column.count("pull_requests_merged", "Merged PRs"),
      column.count("commits_authored", "Commits authored"),
      column.count("commits_committed", "Commits committed"),
      column.count("pull_requests_opened", "Opened PRs"),
      column.count("lines_added", "Lines added"),
      column.count("lines_removed", "Lines removed"),
    ],
    rows,
    empty,
    sort:
      view === "new"
        ? { key: "first", direction: "desc" }
        : { key: "pull_requests_merged", direction: "desc" },
  });
  return `<div class="subtitle contributor-list-count">${number.format(rows.length)} ${view === "new" ? "new " : ""}contributors</div>${table}`;
}

function userList(data, params) {
  const query = params.get("q") || "";
  const days = rangeParam(params, "30");
  const repositoryId = params.get("repository") || "";
  return (
    pageHead("Contributors", "Human contributors with recorded GitHub activity.") +
    filterBar("Contributor list filters", [
      rangeField("contributor-days", days),
      selectField({
        id: "contributor-repository",
        label: "Repository",
        all: "All repositories",
        options: (data.repositories || []).map((repo) => [repo.id, repo.name]),
        selected: repositoryId,
        className: "repo",
      }),
    ]) +
    panel({
      title: "Search",
      body: `<p class="contributor-list-note">New contributors made their first recorded PR, commit, or shared code contribution in the selected range and repository. All contributors were active in that range. Activity and shared line totals cover all tracked dates and repositories.</p>${searchField(
        {
          id: "contributor-search",
          label: "Search by GitHub username or name",
          placeholder: "Username or full name",
          value: query,
          hint: "Matches public profile names, when available, and GitHub usernames.",
        },
      )}`,
    }) +
    panel({
      title: "New contributors",
      id: "new-contributors",
      body: `<div id="new-contributor-results">${contributorTable(data.users, query, "new", days, repositoryId)}</div>`,
    }) +
    panel({
      title: "All contributors",
      id: "all-contributors",
      body: `<div id="all-contributor-results">${contributorTable(data.users, query, "all", days, repositoryId)}</div>`,
    })
  );
}

function bindContributorSearch(data) {
  const input = document.getElementById("contributor-search");
  const days = document.getElementById("contributor-days");
  const repository = document.getElementById("contributor-repository");
  const update = () => {
    for (const view of ["new", "all"]) {
      document.getElementById(`${view}-contributor-results`).innerHTML = contributorTable(
        data.users,
        input.value,
        view,
        days.value,
        repository.value,
      );
    }
    const params = currentRoute().params;
    setParam(params, "q", input.value.trim());
    params.set("days", days.value);
    setParam(params, "repository", repository.value);
    replaceParams(params);
    document.getElementById("page-stamp").textContent = `${rangeStamp(days.value)} · contributors`;
  };
  for (const control of [days, repository]) control.addEventListener("change", update);
  input.addEventListener("input", update);
}

function contributorAiPanel(data) {
  const usage = data.ai_usage || { summary: {}, top_models: [] };
  return panel({
    title: "AI usage",
    subtitle:
      "Explicit AI identities and attribution in this contributor’s commits and pull requests.",
    className: "contributor-ai",
    body: `${stats(usage.summary || {}, [
      ["Commits with AI", "commits_with_ai"],
      ["AI-authored commits", "commits_ai_authored"],
      ["AI co-authored", "commits_ai_coauthored"],
      ["AI assisted", "commits_ai_assisted"],
      ["Pull requests with AI", "pull_requests_with_ai"],
    ])}
    <h3>AI model breakdown</h3>
    <p class="page-copy">Provider names are normalized; distinct models and versions stay separate. Context-window annotations are omitted from model names.</p>
    ${dataTable({
      id: "contributor-ai-models",
      columns: [
        column.text("provider", "Provider"),
        column.text("model", "Model"),
        column.count("commits", "Commits"),
        column.count("pull_requests", "Pull requests"),
        column.count("commits_ai_authored", "AI-authored"),
        column.count("commits_ai_coauthored", "Co-authored"),
        column.count("commits_ai_assisted", "Assisted"),
      ],
      rows: usage.top_models,
      empty: "No explicit AI attribution was found for this contributor in the selected range.",
    })}`,
  });
}

function contributorFilters(data) {
  const filters = data.filters || { days: 365, repository: "" };
  return filterBar("Contributor filters", [
    rangeField("contributor-days", String(filters.days || 365)),
    selectField({
      id: "contributor-repository",
      label: "Repository",
      all: "All repositories",
      options: (data.repository_options || []).map((repository) => [
        repository.id,
        repository.name,
      ]),
      selected: filters.repository || "",
      className: "repo",
    }),
  ]);
}

function repositoryFilters(data) {
  const filters = data.filters || { days: 365, target_branch: "" };
  return filterBar("Repository filters", [
    rangeField("repository-days", String(filters.days || 365)),
    selectField({
      id: "repository-branch",
      label: "Target branch",
      all: "All branches",
      options: (data.repository.target_branches || []).map((branch) => [
        branch.name,
        branch.label ? `${branch.label} (${branch.name})` : branch.name,
      ]),
      selected: filters.target_branch || "",
      className: "repo",
    }),
  ]);
}

function repositoryDetailPage(data) {
  const repo = data.repository;
  return (
    pageHead(repo.name, `${repo.status}${repo.module ? ` · ${repo.module}` : ""}`) +
    repositoryFilters(data) +
    stats(data.summary, [
      ["Merged PRs", "pull_requests_merged"],
      ["Commits", "commits"],
      ["Active contributors", "active_contributors"],
      ["Opened PRs", "pull_requests_opened"],
      ["Lines added", "additions"],
      ["Lines removed", "deletions"],
    ]) +
    languagePanel(
      "Languages",
      data.languages,
      "Current code on the default branch, from GitHub’s language detection",
    ) +
    activityPanel(data.daily) +
    reviewPanel(data.summary, "Merged pull requests in the selected range and branch") +
    `<div id="repository-company-share">${pendingPanel("Contributions by company")}</div>` +
    `<section class="page-grid">${panel({
      title: "Top contributors",
      body: dataTable({
        id: "top-contributors",
        columns: contributorColumns(),
        rows: data.top_contributors,
        empty: "No contributor activity.",
        sort: { key: "pull_requests_merged", direction: "desc" },
      }),
    })}${panel({
      title: "Top reviewers",
      body: dataTable({
        id: "top-reviewers",
        columns: reviewerColumns(),
        rows: data.top_reviewers,
        empty: "No reviews in this range.",
        sort: { key: "reviews", direction: "desc" },
      }),
    })}</section>` +
    panel({
      title: "Recent pull requests",
      body: pullRequestTable("pull-requests", data.recent_pull_requests),
    })
  );
}

function loadRepositoryCompanyShare(repositoryId, filters) {
  const params = new URLSearchParams({
    days: String(filters.days),
    repository: repositoryId,
    target_branch: filters.target_branch || "",
  });
  return loadCompanyShare(
    document.getElementById("repository-company-share"),
    "repository-companies",
    params,
    {
      note: "Shared contribution credits in the selected period and branch. Each contribution counts once for every credited company; Independent is the default when no company affiliation is known.",
    },
  );
}

function userDetailPage(data) {
  const user = data.contributor;
  const historyLoaded = data.history_loaded !== false;
  const historyFailed = Boolean(data.history_error);
  const profile = user.profile_url
    ? externalLink(user.profile_url, `@${user.login || user.name}`)
    : escapeHtml(user.login || user.id);
  const historyPanels = historyLoaded
    ? contributorAiPanel(data) +
      panel({
        title: "Pull requests authored",
        body: pullRequestTable("pull-requests", data.pull_requests),
      }) +
      panel({
        title: "Recent commits for this identity",
        body: commitTable("commits", data.commits),
      })
    : (historyFailed
        ? `<div class="notice" role="alert">${escapeHtml(data.history_error)}</div>`
        : "") +
      pendingPanel("AI usage", historyFailed) +
      pendingPanel("Pull requests authored", historyFailed) +
      pendingPanel("Recent commits for this identity", historyFailed);
  const providerPie = historyLoaded
    ? sharePie({
        id: "contributor-ai-providers",
        title: "Added lines by AI provider",
        rows: data.ai_provider_lines || [],
        metrics: [{ key: "additions", label: "Lines added", unit: "lines added" }],
        link: (row) => escapeHtml(row.name),
        subtitle: "Selected time range",
        note: "Each merged PR is counted once. ‘No provider identified’ means no recognized AI attribution was found in its commit metadata.",
      })
    : pendingPanel("Added lines by AI provider", historyFailed);
  return (
    pageHead(user.name, `${user.kind.toLowerCase()} identity · ${user.kind_source.toLowerCase()}`) +
    `<section class="panel page-section"><div class="page-copy">${profile}${companyProfile(user)}</div></section>` +
    contributorFilters(data) +
    `<p class="notice">Each merged PR’s full line diff is shared with its author and recognized co-authors. Contributor totals overlap.</p>` +
    stats(
      data.summary,
      [
        ["Opened PRs", "pull_requests_opened"],
        ["Merged PRs", "pull_requests_merged"],
        ["Commits authored", "commits_authored"],
        ["Commits committed", "commits_committed"],
        ["Lines added · shared", "shared_additions"],
        ["Lines removed · shared", "shared_deletions"],
        ["PRs reviewed", "reviews"],
      ],
      { pending: !historyLoaded && !historyFailed },
    ) +
    `<section class="page-grid">${repositoryShare(
      "contributor-repositories",
      data.repository_activity || [],
      [
        { ...METRICS.commits, label: "Commits", subtitle: "Commits authored by repository" },
        { ...METRICS.opened, subtitle: "Pull requests opened by repository" },
        {
          ...METRICS.reviews,
          subtitle:
            "Pull requests reviewed by repository, each counted once; reviews of this person’s own pull requests are not counted",
        },
      ],
    )}${providerPie}</section>` +
    languagePanel(
      "Languages",
      data.languages,
      "Languages of the repositories this person committed to, weighted by their commits in each · selected time range",
    ) +
    historyPanels +
    activityPanel(data.daily)
  );
}

function showContributor(contributorId, data) {
  currentContributorData = data;
  content.innerHTML = userDetailPage(data);
  bindContributorFilters(contributorId);
}

async function loadContributorHistory(contributorId, summaryData, requestId) {
  const filters = summaryData.filters || {};
  const query = new URLSearchParams({ days: String(filters.days || "365") });
  if (filters.repository) query.set("repository", filters.repository);
  try {
    const historyData = await getJson(
      `/api/users/${encodePath(contributorId)}/history?${query.toString()}`,
    );
    if (requestId !== page.requestId) return;
    showContributor(contributorId, { ...summaryData, ...historyData });
    document.getElementById("page-stamp").textContent = rangeStamp(
      String(historyData.filters.days),
    );
  } catch (error) {
    if (requestId !== page.requestId) return;
    showContributor(contributorId, {
      ...summaryData,
      history_error: `Detailed activity could not be loaded: ${error.message}`,
    });
  }
}

function bindContributorFilters(contributorId) {
  const days = document.getElementById("contributor-days");
  const repository = document.getElementById("contributor-repository");
  bindFilters([days, repository], {
    apply: (params) => {
      params.set("days", days.value);
      setParam(params, "repository", repository.value);
    },
    url: (params) => `/api/users/${encodePath(contributorId)}?${params.toString()}`,
    render: (data, requestId) => {
      showContributor(contributorId, data);
      document.getElementById("page-stamp").textContent = rangeStamp(String(data.filters.days));
      void loadContributorHistory(contributorId, data, requestId);
    },
    failed: (requestId) => {
      // Finish the previous selection's details if its request was superseded.
      if (
        currentContributorData?.history_loaded === false &&
        !currentContributorData.history_error
      ) {
        void loadContributorHistory(contributorId, currentContributorData, requestId);
      }
    },
  });
}

function aiModelShare(models) {
  return sharePie({
    id: "ai-model-share",
    title: "AI model share",
    rows: models,
    metrics: [
      {
        key: "commits_with_ai",
        label: "Commits",
        unit: "commits",
        subtitle: "Top normalized models by AI-attributed commits",
      },
      {
        key: "pull_requests_with_ai",
        label: "Pull requests",
        unit: "pull requests",
        subtitle: "Top normalized models by pull requests with AI attribution",
      },
    ],
    name: (row) => (row.model ? `${row.provider} · ${row.model}` : row.provider),
    link: (row) =>
      aiContributorLink(
        row.provider_key,
        row.provider,
        "",
        row.provider,
        row.model ? `${row.provider} · ${row.model}` : row.provider,
      ),
    note: "A commit or pull request with multiple AI identities contributes to each matching model.",
    empty: "No recognized AI model activity in this range.",
  });
}

function aiContributorDetailPage(data) {
  const identity = data.identity || {};
  const heading = identity.provider || identity.name || "AI provider";
  const models = data.models || [];
  // Point releases are grouped under the major version; list the exact versions seen.
  const tracked = models
    .map((model) => {
      const versions = model.versions || [];
      const label = model.model || model.name || model.provider;
      return versions.length ? `${label} (${versions.join(", ")})` : label;
    })
    .join(", ");
  return (
    pageHead(heading, "AI provider activity aggregated across its recognized models") +
    `<section class="panel page-section"><div class="page-copy"><strong>Provider:</strong> ${escapeHtml(identity.provider || "Other")}<br><strong>Models tracked:</strong> ${number.format(models.length)}${tracked ? ` · ${escapeHtml(tracked)}` : ""}</div></section>` +
    contributorFilters(data) +
    stats(data.summary || {}, [
      ["Commits with AI", "commits_with_ai"],
      ["AI-authored commits", "commits_ai_authored"],
      ["AI co-authored commits", "commits_ai_coauthored"],
      ["AI-assisted commits", "commits_ai_assisted"],
      ["Pull requests with AI", "pull_requests_with_ai"],
      ["Lines added · shared", "shared_additions"],
      ["Lines removed · shared", "shared_deletions"],
    ]) +
    `<p class="notice">Line changes are fully shared with each recognized AI participant; provider totals can overlap across providers.</p>` +
    aiModelShare(data.models || []) +
    repositoryShare("ai-contributor-repositories", data.repository_activity || [], [
      { ...METRICS.commits, unit: "AI commits", subtitle: "AI-attributed commits by repository" },
      {
        key: "pull_requests_with_ai",
        label: "Pull requests",
        unit: "PRs with AI",
        subtitle: "AI-attributed pull requests by repository",
      },
    ]) +
    activityPanel(data.daily, {
      title: "AI activity over time",
      subtitle: "Daily AI-attributed pull requests and commits, UTC",
      series: [
        { key: "pull_requests_merged", label: "Pull requests with AI" },
        { key: "commits", label: "AI-attributed commits" },
      ],
    }) +
    panel({
      title: "Recent AI-attributed pull requests",
      body: pullRequestTable("ai-pull-requests", data.recent_pull_requests || [], {
        author: false,
        state: false,
        attribution: true,
        empty: "No AI-attributed pull requests in this range.",
      }),
    }) +
    panel({
      title: "Recent AI-attributed commits",
      body: commitTable("ai-commits", data.recent_commits || []),
    })
  );
}

function bindAIContributorFilters(identityKey) {
  const days = document.getElementById("contributor-days");
  const repository = document.getElementById("contributor-repository");
  bindFilters([days, repository], {
    apply: (params) => {
      params.set("days", days.value);
      setParam(params, "repository", repository.value);
    },
    url: (params) => `/api/ai-contributors/${encodeURIComponent(identityKey)}?${params.toString()}`,
    render: (data) => {
      content.innerHTML = aiContributorDetailPage(data);
      bindAIContributorFilters(identityKey);
      document.getElementById("page-stamp").textContent = rangeStamp(String(data.filters.days));
    },
  });
}

function bindRepositoryFilters(repositoryId) {
  const days = document.getElementById("repository-days");
  const branch = document.getElementById("repository-branch");
  bindFilters([days, branch], {
    apply: (params) => {
      params.set("days", days.value);
      setParam(params, "target_branch", branch.value);
    },
    url: (params) => `/api/repositories/${encodePath(repositoryId)}?${params.toString()}`,
    render: (data) => {
      content.innerHTML = repositoryDetailPage(data);
      bindRepositoryFilters(repositoryId);
      void loadRepositoryCompanyShare(repositoryId, data.filters);
      document.getElementById("page-stamp").textContent = rangeStamp(String(data.filters.days));
    },
  });
}

function aiActivityPage(data) {
  const summary = data.summary;
  return (
    pageHead(
      "AI Activity",
      "Explicit AI authorship and assistance found in collected pull requests and commit trailers.",
    ) +
    stats(summary, [
      ["Commits with AI", "commits_with_ai"],
      ["AI-authored commits", "commits_ai_authored"],
      ["AI co-authored commits", "commits_ai_coauthored"],
      ["AI-assisted commits", "commits_ai_assisted"],
      ["PRs with AI", "pull_requests_with_ai"],
      ["AI-authored PRs", "pull_requests_ai_authored"],
      ["PRs with an AI-authored commit", "pull_requests_with_ai_authored_commit"],
      ["PRs with AI co-author", "pull_requests_ai_coauthored"],
      ["PRs with AI assistance", "pull_requests_ai_assisted"],
    ]) +
    `<p class="notice">${escapeHtml(data.recognition)} Counts can overlap when a commit or PR has more than one AI attribution. Unrecognized trailers are shown as context: ${number.format(summary.commits_with_coauthor_trailer)} commits have a Co-Authored-By trailer and ${number.format(summary.commits_with_assisted_trailer)} have an Assisted-By trailer.</p>` +
    panel({
      title: "AI providers",
      subtitle:
        "Provider totals count each commit and pull request once across that provider’s models. Line changes are shared across recognized AI participants.",
      body: dataTable({
        id: "ai-providers",
        columns: aiProviderColumns(),
        rows: data.top_ai_providers,
        empty: "No recognized AI provider activity in this range.",
      }),
    }) +
    aiModelShare(data.top_ais || []) +
    panel({
      title: "AI model breakdown",
      subtitle:
        "Model variants remain distinct; context-window annotations are normalized away. Provider-only tools have no model name.",
      body: dataTable({
        id: "ai-models",
        columns: aiModelColumns(),
        rows: data.top_ais,
        empty: "No AI models have activity in this range.",
      }),
    }) +
    panel({
      title: "AI activity by repository",
      body: dataTable({
        id: "ai-repositories",
        columns: [
          repositoryColumn(),
          column.count("commits_ai_authored", "AI-authored commits"),
          column.count("commits_ai_coauthored", "AI co-authored commits"),
          column.count("commits_ai_assisted", "AI-assisted commits"),
          column.count("pull_requests_ai_authored", "AI-authored PRs"),
          column.count("pull_requests_with_ai_authored_commit", "PRs with an AI-authored commit"),
          column.count("pull_requests_ai_coauthored", "PRs with AI co-author"),
          column.count("pull_requests_ai_assisted", "PRs with AI assistance"),
        ],
        rows: data.by_repository,
        empty: "No AI attribution has been identified yet.",
      }),
    }) +
    panel({
      title: "Recent AI-attributed commits",
      body: commitTable("ai-commits", data.recent_commits),
    }) +
    panel({
      title: "Recent AI-attributed pull requests",
      body: pullRequestTable("ai-pull-requests", data.recent_pull_requests, {
        state: false,
        attribution: true,
        empty: "No AI-attributed pull requests.",
      }),
    })
  );
}

// Render one detail route into #page-content. `requestId` is the navigation's page.requestId;
// a newer navigation makes this render stop before replacing the content.
export async function loadBrowsePage(route, requestId) {
  const { parts, params } = route;
  const section = parts[0] || "";
  // Page paths span several segments: #/users/github/octocat, #/repositories/github/owner/name.
  const id = parts.slice(1).join("/");
  const stale = () => requestId !== page.requestId;
  const withDays = () => {
    const query = new URLSearchParams(params);
    if (!query.has("days")) query.set("days", "365");
    return query;
  };
  let data;
  if (section === "repositories" && !id) {
    data = await getJson("/api/repositories");
    if (stale()) return;
    content.innerHTML = repositoryList(data);
  } else if (section === "repositories") {
    data = await getJson(`/api/repositories/${encodePath(id)}?${withDays()}`);
    if (stale()) return;
    content.innerHTML = repositoryDetailPage(data);
    bindRepositoryFilters(id);
    void loadRepositoryCompanyShare(id, data.filters);
  } else if (section === "users" && !id) {
    data = await getJson("/api/users");
    if (stale()) return;
    content.innerHTML = userList(data, params);
    bindContributorSearch(data);
  } else if (section === "users") {
    data = await getJson(`/api/users/${encodePath(id)}?${withDays()}`);
    if (stale()) return;
    showContributor(id, data);
    void loadContributorHistory(id, data, page.requestId);
  } else if (section === "companies") {
    data = await getJson(`/api/companies${id ? `/${encodeURIComponent(id)}` : ""}?${withDays()}`);
    if (stale()) return;
    content.innerHTML = id ? companyDetailPage(data) : companyList(data);
    bindCompanyFilters(data, id);
  } else if (section === "ai-activity") {
    data = await getJson("/api/ai-usage");
    if (stale()) return;
    content.innerHTML = aiActivityPage(data);
  } else if (section === "ai-contributors" && id) {
    data = await getJson(`/api/ai-contributors/${encodeURIComponent(id)}?${withDays()}`);
    if (stale()) return;
    content.innerHTML = aiContributorDetailPage(data);
    bindAIContributorFilters(id);
  } else {
    throw new Error("Page not found");
  }
  if (
    section === "companies" ||
    (id && ["repositories", "users", "ai-contributors"].includes(section))
  ) {
    document.getElementById("page-stamp").textContent = rangeStamp(String(data.filters.days));
  } else if (section === "users" && !id) {
    document.getElementById("page-stamp").textContent =
      `${rangeStamp(rangeParam(params, "30"))} · contributors`;
    if (params.get("section") === "new-contributors") {
      document.getElementById("new-contributors").scrollIntoView();
    }
  } else {
    document.getElementById("page-stamp").textContent = "Last 365 days unless noted";
  }
}
