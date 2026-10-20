import { getJson } from "./api.js";
import {
  activityPanel,
  aiContributorLink,
  aiIdentityLinks,
  chartColor,
  content,
  contributionPie,
  errorBox,
  escapeHtml,
  externalLink,
  formatDate,
  number,
  page,
  pageHead,
  pendingPanel,
  rangeStamp,
  repositoryLink,
  setPageLoading,
  stats,
  table,
  userLink,
} from "./common.js";
import {
  bindCompanyFilters,
  companyDetailPage,
  companyList,
  companyProfile,
  loadRepositoryCompanyShare,
} from "./companies.js";
import { currentRoute, replaceParams } from "./route.js";

let currentContributorData = null;

const repositorySortColumns = [
  ["name", "Repository"],
  ["status", "Status"],
  ["merged", "Merged PRs"],
  ["commits", "Commits"],
  ["contributors", "Contributors"],
];

function repositorySortState(params) {
  const key = params.get("repo_sort");
  const direction = params.get("repo_dir");
  return {
    key: repositorySortColumns.some(([candidate]) => candidate === key) ? key : "merged",
    direction: ["asc", "desc"].includes(direction) ? direction : "desc",
  };
}

function repositoryTable(repositories, sort) {
  if (!repositories.length)
    return '<div class="empty">No repositories are in the catalog yet.</div>';
  const value = (item) => {
    if (sort.key === "name" || sort.key === "status") return item[sort.key] || "";
    const summaryKey = {
      merged: "pull_requests_merged",
      commits: "commits",
      contributors: "active_contributors",
    }[sort.key];
    return Number(item.summary[summaryKey]) || 0;
  };
  const ordered = [...repositories].sort((a, b) => {
    const aValue = value(a);
    const bValue = value(b);
    const comparison =
      typeof aValue === "number" ? aValue - bValue : String(aValue).localeCompare(String(bValue));
    return (sort.direction === "asc" ? comparison : -comparison) || a.name.localeCompare(b.name);
  });
  const rows = ordered.map(
    (item) => `<tr>
    <td class="table-link">${repositoryLink(item.id, item.name, item.target_branch)}</td>
    <td>${escapeHtml(item.status)}</td>
    <td class="count">${number.format(item.summary.pull_requests_merged)}</td>
    <td class="count">${number.format(item.summary.commits)}</td>
    <td class="count">${number.format(item.summary.active_contributors)}</td>
  </tr>`,
  );
  const headings = repositorySortColumns
    .map(
      ([key, label]) =>
        `<th aria-sort="${sort.key === key ? (sort.direction === "asc" ? "ascending" : "descending") : "none"}"><button type="button" class="sort-button" data-repository-sort="${key}">${label}${sort.key === key ? (sort.direction === "asc" ? " ↑" : " ↓") : ""}</button></th>`,
    )
    .join("");
  return `<div class="table-wrap"><table><thead><tr>${headings}</tr></thead><tbody>${rows.join("")}</tbody></table></div>`;
}

function repositoryList(data) {
  const sort = repositorySortState(currentRoute().params);
  return (
    pageHead(
      "Repositories",
      "Activity across tracked repositories and target branches, based on the last 365 days.",
    ) +
    `<section class="panel page-section" id="repository-results">${repositoryTable(data.repositories, sort)}</section>`
  );
}

function bindRepositorySort(data) {
  const results = document.getElementById("repository-results");
  const sort = repositorySortState(currentRoute().params);
  results.addEventListener("click", (event) => {
    const button = event.target.closest("[data-repository-sort]");
    if (!button) return;
    const selected = button.dataset.repositorySort;
    if (sort.key === selected) sort.direction = sort.direction === "asc" ? "desc" : "asc";
    else {
      sort.key = selected;
      sort.direction = ["name", "status"].includes(selected) ? "asc" : "desc";
    }
    const params = currentRoute().params;
    params.set("repo_sort", sort.key);
    params.set("repo_dir", sort.direction);
    replaceParams(params);
    results.innerHTML = repositoryTable(data.repositories, sort);
  });
}

const contributorSortKeys = new Set([
  "name",
  "first",
  "merged",
  "authored",
  "committed",
  "opened",
  "added",
  "removed",
]);

function contributorSortState(params, view) {
  const sortParam = view === "new" ? "new_sort" : "sort";
  const directionParam = view === "new" ? "new_dir" : "dir";
  const key = contributorSortKeys.has(params.get(sortParam))
    ? params.get(sortParam)
    : view === "new"
      ? "first"
      : "merged";
  const direction = ["asc", "desc"].includes(params.get(directionParam))
    ? params.get(directionParam)
    : key === "name"
      ? "asc"
      : "desc";
  return { key, direction };
}

function contributorTable(users, query, view, days, repositoryId, sortKey, sortDirection) {
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const today = new Date().toISOString().slice(0, 10);
  const start =
    days === "all"
      ? ""
      : new Date(Date.now() - (Number(days) - 1) * 86400000).toISOString().slice(0, 10);
  const matchingUsers = users.filter((item) => {
    const searchable = `${item.login || ""} ${item.name || ""}`.toLocaleLowerCase();
    if (!searchable.includes(normalizedQuery)) return false;
    const contributionDate =
      view === "new"
        ? repositoryId
          ? item.first_contribution_by_repository?.[repositoryId]
          : item.first_contribution_date
        : repositoryId
          ? item.last_contribution_by_repository?.[repositoryId]
          : item.last_contribution_date;
    if (!contributionDate) return days === "all" && !repositoryId && view !== "new";
    return contributionDate >= start && contributionDate <= today;
  });
  const firstDate = (item) =>
    repositoryId
      ? item.first_contribution_by_repository?.[repositoryId] || ""
      : item.first_contribution_date || "";
  const sortField = {
    merged: "pull_requests_merged",
    authored: "commits_authored",
    committed: "commits_committed",
    opened: "pull_requests_opened",
    added: "lines_added",
    removed: "lines_removed",
  }[sortKey];
  matchingUsers.sort((a, b) => {
    const aValue = sortKey === "name" ? a.name : sortKey === "first" ? firstDate(a) : a[sortField];
    const bValue = sortKey === "name" ? b.name : sortKey === "first" ? firstDate(b) : b[sortField];
    const comparison =
      typeof aValue === "number"
        ? aValue - bValue
        : String(aValue || "").localeCompare(String(bValue || ""));
    return (sortDirection === "asc" ? comparison : -comparison) || a.name.localeCompare(b.name);
  });
  const rows = matchingUsers.map(
    (item) => `<tr>
    <td class="table-link">${userLink(item.id, item.name, item.login)}</td>
    <td>${formatDate(
      repositoryId
        ? item.first_contribution_by_repository?.[repositoryId]
        : item.first_contribution_date,
    )}</td>
    <td class="count">${number.format(item.pull_requests_merged)}</td>
    <td class="count">${number.format(item.commits_authored)}</td>
    <td class="count">${number.format(item.commits_committed)}</td>
    <td class="count">${number.format(item.pull_requests_opened)}</td>
    <td class="count">${number.format(item.lines_added || 0)}</td>
    <td class="count">${number.format(item.lines_removed || 0)}</td>
  </tr>`,
  );
  const emptyText = normalizedQuery
    ? `No contributors match “${query.trim()}”.`
    : view === "new"
      ? "No new contributors in this range."
      : "No contributor activity in this range.";
  const headers = [
    ["name", "Contributor"],
    ["first", "First recorded"],
    ["merged", "Merged PRs"],
    ["authored", "Commits authored"],
    ["committed", "Commits committed"],
    ["opened", "Opened PRs"],
    ["added", "Lines added"],
    ["removed", "Lines removed"],
  ];
  const headings = headers
    .map(
      ([key, label]) =>
        `<th aria-sort="${sortKey === key ? (sortDirection === "asc" ? "ascending" : "descending") : "none"}"><button type="button" class="sort-button" data-contributor-sort="${key}" data-contributor-view="${view}">${label}<span aria-hidden="true">${sortKey === key ? (sortDirection === "asc" ? " ↑" : " ↓") : ""}</span></button></th>`,
    )
    .join("");
  // New contributors can be many in a long range; that list scrolls after ten rows.
  const wrapClass = view === "new" ? "table-wrap table-scroll" : "table-wrap";
  const body = rows.length
    ? `<div class="${wrapClass}"><table><thead><tr>${headings}</tr></thead><tbody>${rows.join("")}</tbody></table></div>`
    : `<div class="empty">${escapeHtml(emptyText)}</div>`;
  return `<div class="subtitle contributor-list-count">${number.format(matchingUsers.length)} ${view === "new" ? "new " : ""}contributors</div>${body}`;
}

function userList(data, params) {
  const query = params.get("q") || "";
  const days = ["7", "30", "90", "365", "all"].includes(params.get("days"))
    ? params.get("days")
    : "30";
  const repositoryId = params.get("repository_id") || "";
  const newSort = contributorSortState(params, "new");
  const allSort = contributorSortState(params, "all");
  const repositoryOptions = (data.repositories || [])
    .map(
      (repo) =>
        `<option value="${escapeHtml(repo.id)}"${repo.id === repositoryId ? " selected" : ""}>${escapeHtml(repo.name)}</option>`,
    )
    .join("");
  return (
    pageHead("Contributors", "Human contributors with recorded GitHub activity.") +
    `<section class="panel page-section">
      <div class="filters contributor-list-filters" aria-label="Contributor list filters">
        <div class="field"><label for="contributor-days">Time range</label><select id="contributor-days"><option value="7"${days === "7" ? " selected" : ""}>Last 7 days</option><option value="30"${days === "30" ? " selected" : ""}>Last 30 days</option><option value="90"${days === "90" ? " selected" : ""}>Last 90 days</option><option value="365"${days === "365" ? " selected" : ""}>Last 365 days</option><option value="all"${days === "all" ? " selected" : ""}>All available</option></select></div>
        <div class="field repo"><label for="contributor-repository">Repository</label><select id="contributor-repository"><option value="">All repositories</option>${repositoryOptions}</select></div>
      </div>
      <p class="contributor-list-note">New contributors made their first recorded PR, commit, or shared code contribution in the selected range and repository. All contributors were active in that range. Activity and shared line totals cover all tracked dates and repositories.</p>
      <div class="search-control"><label for="contributor-search">Search by GitHub username or name</label>
      <input id="contributor-search" type="search" value="${escapeHtml(query)}" placeholder="Username or full name" autocomplete="off">
      <div class="subtitle">Matches public profile names, when available, and GitHub usernames.</div></div>
    </section>
    <section class="panel page-section" id="new-contributors"><h2>New contributors</h2><div id="new-contributor-results">${contributorTable(data.users, query, "new", days, repositoryId, newSort.key, newSort.direction)}</div></section>
    <section class="panel page-section" id="all-contributors"><h2>All contributors</h2><div id="all-contributor-results">${contributorTable(data.users, query, "all", days, repositoryId, allSort.key, allSort.direction)}</div></section>`
  );
}

function bindContributorSearch(data) {
  const input = document.getElementById("contributor-search");
  const newResults = document.getElementById("new-contributor-results");
  const allResults = document.getElementById("all-contributor-results");
  const days = document.getElementById("contributor-days");
  const repository = document.getElementById("contributor-repository");
  const params = currentRoute().params;
  const sorts = {
    new: contributorSortState(params, "new"),
    all: contributorSortState(params, "all"),
  };
  const update = () => {
    newResults.innerHTML = contributorTable(
      data.users,
      input.value,
      "new",
      days.value,
      repository.value,
      sorts.new.key,
      sorts.new.direction,
    );
    allResults.innerHTML = contributorTable(
      data.users,
      input.value,
      "all",
      days.value,
      repository.value,
      sorts.all.key,
      sorts.all.direction,
    );
    const next = currentRoute().params;
    if (input.value.trim()) next.set("q", input.value.trim());
    else next.delete("q");
    next.delete("view");
    next.set("days", days.value);
    if (repository.value) next.set("repository_id", repository.value);
    else next.delete("repository_id");
    next.set("new_sort", sorts.new.key);
    next.set("new_dir", sorts.new.direction);
    next.set("sort", sorts.all.key);
    next.set("dir", sorts.all.direction);
    replaceParams(next);
    document.getElementById("page-stamp").textContent = `${rangeStamp(days.value)} · contributors`;
  };
  for (const control of [days, repository]) control.addEventListener("change", update);
  input.addEventListener("input", update);
  for (const results of [newResults, allResults])
    results.addEventListener("click", (event) => {
      const button = event.target.closest("[data-contributor-sort]");
      if (!button) return;
      const selected = button.dataset.contributorSort;
      const sort = sorts[button.dataset.contributorView];
      if (sort.key === selected) sort.direction = sort.direction === "asc" ? "desc" : "asc";
      else {
        sort.key = selected;
        sort.direction = selected === "name" ? "asc" : "desc";
      }
      update();
    });
}

function pullRequestTable(rows) {
  const tableRows = rows.map(
    (item) => `<tr>
    <td>${formatDate(item.created_at)}</td>
    <td class="table-link">${externalLink(item.url, `#${item.number} ${item.title}`)}</td>
    <td>${repositoryLink(item.repository_id, item.repository, item.target_branch)}</td>
    <td>${escapeHtml(item.target_branch || "—")}</td>
    <td>${userLink(item.author_id, item.author_name, item.author_login)}</td>
    <td>${escapeHtml(item.state)}</td>
  </tr>`,
  );
  return table(
    ["Created", "Pull request", "Repository", "Target branch", "Author", "State"],
    tableRows,
    "No pull requests to show.",
  );
}

function commitTable(rows) {
  const tableRows = rows.map(
    (item) => `<tr>
    <td>${formatDate(item.committed_at)}</td>
    <td class="table-link">${externalLink(item.url, item.subject || item.message || item.sha)}</td>
    <td>${repositoryLink(item.repository_id, item.repository)}</td>
    <td>${(item.target_branches || []).map((branch) => repositoryLink(item.repository_id, branch, branch)).join(" ") || "—"}</td>
    <td>${escapeHtml((item.categories || []).join(", ") || item.role || "Author")}${item.author_name ? `<small>Author: ${userLink(item.author_id, item.author_name, item.author_login)}</small>` : ""}${item.committer_name ? `<small>Committer: ${userLink(item.committer_id, item.committer_name, item.committer_login)}</small>` : ""}${item.ai_coauthors?.length ? `<small>Co-Authored-By: ${aiIdentityLinks(item.ai_coauthors)}</small>` : ""}${item.ai_assistants?.length ? `<small>Assisted-By: ${aiIdentityLinks(item.ai_assistants)}</small>` : ""}</td>
  </tr>`,
  );
  return table(
    ["Committed", "Commit", "Repository", "Target branches", "Attribution"],
    tableRows,
    "No commits to show.",
  );
}

function contributorAiPanel(data) {
  const usage = data.ai_usage || { summary: {}, top_models: [] };
  const summary = usage.summary || {};
  const modelRows = (usage.top_models || []).map(
    (item) => `<tr>
    <td>${escapeHtml(item.provider)}</td>
    <td>${escapeHtml(item.model || "—")}</td>
    <td class="count">${number.format(item.commits)}</td>
    <td class="count">${number.format(item.pull_requests)}</td>
    <td class="count">${number.format(item.commits_ai_authored)}</td>
    <td class="count">${number.format(item.commits_ai_coauthored)}</td>
    <td class="count">${number.format(item.commits_ai_assisted)}</td>
  </tr>`,
  );
  return `<section class="panel page-section contributor-ai">
    <div class="panel-head"><div><h2>AI usage</h2><div class="subtitle">Explicit AI identities and attribution in this contributor’s commits and pull requests.</div></div></div>
    ${stats(summary, [
      ["Commits with AI", "commits_with_ai"],
      ["AI-authored commits", "commits_ai_authored"],
      ["AI co-authored", "commits_ai_coauthored"],
      ["AI assisted", "commits_ai_assisted"],
      ["Pull requests with AI", "pull_requests_with_ai"],
    ])}
    <h3>AI model breakdown</h3>
    <p class="page-copy">Provider names are normalized; distinct models and versions stay separate. Context-window annotations are omitted from model names.</p>
    ${table(["Provider", "Model", "Commits", "Pull requests", "AI-authored", "Co-authored", "Assisted"], modelRows, "No explicit AI attribution was found for this contributor in the selected range.")}
  </section>`;
}

function contributorFilters(data) {
  const filters = data.filters || { days: 365, repository_id: "" };
  const selectedDays = String(filters.days || 365);
  const selectedRepository = filters.repository_id || "";
  const dayOptions = [
    ["7", "Last 7 days"],
    ["30", "Last 30 days"],
    ["90", "Last 90 days"],
    ["365", "Last 365 days"],
    ["all", "All available data"],
  ];
  const repositoryOptions = (data.repository_options || [])
    .map(
      (repository) =>
        `<option value="${escapeHtml(repository.id)}"${repository.id === selectedRepository ? " selected" : ""}>${escapeHtml(repository.name)}</option>`,
    )
    .join("");
  return `<section class="filters contributor-filters" aria-label="Contributor filters">
    <div class="field"><label for="contributor-days">Time range</label><select id="contributor-days">${dayOptions.map(([value, label]) => `<option value="${value}"${value === selectedDays ? " selected" : ""}>${label}</option>`).join("")}</select></div>
    <div class="field contributor-repository-filter"><label for="contributor-repository">Repository</label><select id="contributor-repository"><option value="">All repositories</option>${repositoryOptions}</select></div>
  </section>`;
}

function repositoryDetailPage(data) {
  const repo = data.repository;
  const contributors = data.top_contributors.map(
    (item) =>
      `<tr><td>${userLink(item.id, item.name, item.login)}</td><td class="count">${number.format(item.pull_requests_merged)}</td><td class="count">${number.format(item.commits)}</td></tr>`,
  );
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
    activityPanel(data.daily) +
    `<div id="repository-company-share">${pendingPanel("Contributions by company")}</div>` +
    `<section class="panel page-section"><h2>Top contributors</h2>${table(["Contributor", "Merged PRs", "Commits"], contributors, "No contributor activity.")}</section>
    <section class="panel page-section"><h2>Recent pull requests</h2>${pullRequestTable(data.recent_pull_requests)}</section>`
  );
}

function repositoryFilters(data) {
  const repo = data.repository;
  const filters = data.filters || { days: 365, target_branch: "" };
  const selectedDays = String(filters.days || 365);
  const selectedBranch = filters.target_branch || "";
  const dayOptions = [
    ["7", "Last 7 days"],
    ["30", "Last 30 days"],
    ["90", "Last 90 days"],
    ["365", "Last 365 days"],
    ["all", "All available data"],
  ];
  const branchOptions = (repo.target_branches || [])
    .map((branch) => {
      const label = branch.label ? `${branch.label} (${branch.name})` : branch.name;
      return `<option value="${escapeHtml(branch.name)}"${branch.name === selectedBranch ? " selected" : ""}>${escapeHtml(label)}</option>`;
    })
    .join("");
  return `<section class="filters repository-filters" aria-label="Repository filters">
    <div class="field"><label for="repository-days">Time range</label><select id="repository-days">${dayOptions.map(([value, label]) => `<option value="${value}"${value === selectedDays ? " selected" : ""}>${label}</option>`).join("")}</select></div>
    <div class="field repository-branch-filter"><label for="repository-branch">Target branch</label><select id="repository-branch"><option value=""${selectedBranch ? "" : " selected"}>All branches</option>${branchOptions}</select></div>
  </section>`;
}

function userDetailPage(data) {
  const user = data.contributor;
  const historyLoaded = data.history_loaded !== false;
  const historyFailed = Boolean(data.history_error);
  const company = companyProfile(user);
  const profile = user.profile_url
    ? externalLink(user.profile_url, `@${user.login || user.name}`)
    : escapeHtml(user.login || user.id);
  const historyPanels = historyLoaded
    ? contributorAiPanel(data) +
      `<section class="panel page-section"><h2>Pull requests authored</h2>${pullRequestTable(data.pull_requests)}</section>
      <section class="panel page-section"><h2>Recent commits for this identity</h2>${commitTable(data.commits)}</section>`
    : (historyFailed
        ? `<div class="notice" role="alert">${escapeHtml(data.history_error)}</div>`
        : "") +
      pendingPanel("AI usage", historyFailed) +
      pendingPanel("Pull requests authored", historyFailed) +
      pendingPanel("Recent commits for this identity", historyFailed);
  const providerPie = historyLoaded
    ? contributionPie("Added lines by AI provider", data.ai_provider_lines || [], "additions", {
        linkRepositories: false,
        subtitle:
          "Each merged PR is counted once. ‘No provider identified’ means no recognized AI attribution was found in its commit metadata.",
      })
    : pendingPanel("Added lines by AI provider", historyFailed);
  return (
    pageHead(user.name, `${user.kind.toLowerCase()} identity · ${user.kind_source.toLowerCase()}`) +
    `<section class="panel page-section"><div class="page-copy">${profile}${company}</div></section>` +
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
      ],
      !historyLoaded && !historyFailed,
    ) +
    `<section class="page-grid contributor-repository-pies">${contributionPie("Commits authored by repository", data.repository_activity || [], "commits")}${contributionPie("Pull requests opened by repository", data.repository_activity || [], "pull_requests_opened")}${providerPie}</section>` +
    historyPanels +
    activityPanel(data.daily) +
    ""
  );
}

async function loadContributorHistory(contributorId, summaryData, requestId) {
  const filters = summaryData.filters || {};
  const query = new URLSearchParams({ days: String(filters.days || "365") });
  if (filters.repository_id) query.set("repository_id", filters.repository_id);
  try {
    const historyData = await getJson(
      `/api/users/${encodeURIComponent(contributorId)}/history?${query.toString()}`,
    );
    if (requestId !== page.requestId) return;
    currentContributorData = { ...summaryData, ...historyData };
    content.innerHTML = userDetailPage(currentContributorData);
    bindContributorFilters(contributorId);
    document.getElementById("page-stamp").textContent = rangeStamp(
      String(historyData.filters.days),
    );
  } catch (error) {
    if (requestId !== page.requestId) return;
    currentContributorData = {
      ...summaryData,
      history_error: `Detailed activity could not be loaded: ${error.message}`,
    };
    content.innerHTML = userDetailPage(currentContributorData);
    bindContributorFilters(contributorId);
  }
}

function bindContributorFilters(contributorId) {
  const daysSelect = document.getElementById("contributor-days");
  const repositorySelect = document.getElementById("contributor-repository");
  const update = async () => {
    const requestId = ++page.requestId;
    const params = currentRoute().params;
    params.set("days", daysSelect.value);
    if (repositorySelect.value) params.set("repository_id", repositorySelect.value);
    else params.delete("repository_id");
    replaceParams(params);
    setPageLoading(true);
    errorBox.style.display = "none";
    try {
      const data = await getJson(
        `/api/users/${encodeURIComponent(contributorId)}?${params.toString()}`,
      );
      if (requestId !== page.requestId) return;
      currentContributorData = data;
      content.innerHTML = userDetailPage(data);
      bindContributorFilters(contributorId);
      document.getElementById("page-stamp").textContent = rangeStamp(String(data.filters.days));
      void loadContributorHistory(contributorId, data, requestId);
    } catch (error) {
      if (requestId !== page.requestId) return;
      errorBox.textContent = `Could not update activity; showing previous results. ${error.message}`;
      errorBox.style.display = "block";
      if (
        currentContributorData?.history_loaded === false &&
        !currentContributorData.history_error
      ) {
        // Finish the previous selection's details if its request was superseded.
        void loadContributorHistory(contributorId, currentContributorData, requestId);
      }
    } finally {
      if (requestId === page.requestId) setPageLoading(false);
    }
  };
  daysSelect.addEventListener("change", update);
  repositorySelect.addEventListener("change", update);
}

function aiContributorDetailPage(data) {
  const identity = data.identity || {};
  const heading = identity.provider || identity.name || "AI provider";
  const description = `AI provider activity aggregated across its recognized models`;
  const pullRequestRows = (data.recent_pull_requests || []).map(
    (item) => `<tr>
    <td>${formatDate(item.created_at)}</td>
    <td class="table-link">${externalLink(item.url, `#${item.number} ${item.title}`)}</td>
    <td>${repositoryLink(item.repository_id, item.repository, item.target_branch)}</td>
    <td>${escapeHtml(item.target_branch || "—")}</td>
    <td>${escapeHtml((item.categories || []).join(", ") || "AI attribution")}</td>
  </tr>`,
  );
  return (
    pageHead(heading, description) +
    `<section class="panel page-section"><div class="page-copy"><strong>Provider:</strong> ${escapeHtml(identity.provider || "Other")}<br><strong>Models tracked:</strong> ${number.format((data.models || []).length)}</div></section>` +
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
    aiModelSharePanel() +
    `<section class="page-grid contributor-repository-pies">${contributionPie("AI-attributed commits by repository", data.repository_activity || [], "commits")}${contributionPie("AI-attributed pull requests by repository", data.repository_activity || [], "pull_requests_with_ai")}</section>` +
    activityPanel(data.daily, {
      title: "AI activity over time",
      subtitle: "Daily AI-attributed pull requests and commits, UTC",
      firstKey: "pull_requests_merged",
      secondKey: "commits",
      firstLabel: "Pull requests with AI",
      secondLabel: "AI-attributed commits",
    }) +
    `<section class="panel page-section"><h2>Recent AI-attributed pull requests</h2>${table(["Created", "Pull request", "Repository", "Target branch", "AI attribution"], pullRequestRows, "No AI-attributed pull requests in this range.")}</section>
    <section class="panel page-section"><h2>Recent AI-attributed commits</h2>${commitTable(data.recent_commits || [])}</section>`
  );
}

function bindAIContributorFilters(identityKey) {
  const daysSelect = document.getElementById("contributor-days");
  const repositorySelect = document.getElementById("contributor-repository");
  const update = async () => {
    const requestId = ++page.requestId;
    const params = currentRoute().params;
    params.set("days", daysSelect.value);
    if (repositorySelect.value) params.set("repository_id", repositorySelect.value);
    else params.delete("repository_id");
    replaceParams(params);
    setPageLoading(true);
    errorBox.style.display = "none";
    try {
      const data = await getJson(
        `/api/ai-contributors/${encodeURIComponent(identityKey)}?${params.toString()}`,
      );
      if (requestId !== page.requestId) return;
      content.innerHTML = aiContributorDetailPage(data);
      bindAIContributorFilters(identityKey);
      bindAIModelShare(data.models || []);
      document.getElementById("page-stamp").textContent = rangeStamp(String(data.filters.days));
    } catch (error) {
      if (requestId !== page.requestId) return;
      errorBox.textContent = `Could not update activity; showing previous results. ${error.message}`;
      errorBox.style.display = "block";
    } finally {
      if (requestId === page.requestId) setPageLoading(false);
    }
  };
  daysSelect.addEventListener("change", update);
  repositorySelect.addEventListener("change", update);
}

function bindRepositoryFilters(repositoryId) {
  const daysSelect = document.getElementById("repository-days");
  const branchSelect = document.getElementById("repository-branch");
  const update = async () => {
    const requestId = ++page.requestId;
    const params = currentRoute().params;
    params.set("days", daysSelect.value);
    if (branchSelect.value) params.set("target_branch", branchSelect.value);
    else params.delete("target_branch");
    replaceParams(params);
    setPageLoading(true);
    errorBox.style.display = "none";
    try {
      const data = await getJson(
        `/api/repositories/${encodeURIComponent(repositoryId)}?${params.toString()}`,
      );
      if (requestId !== page.requestId) return;
      content.innerHTML = repositoryDetailPage(data);
      bindRepositoryFilters(repositoryId);
      void loadRepositoryCompanyShare(repositoryId, data.filters);
      document.getElementById("page-stamp").textContent = rangeStamp(String(data.filters.days));
    } catch (error) {
      if (requestId !== page.requestId) return;
      errorBox.textContent = `Could not update activity; showing previous results. ${error.message}`;
      errorBox.style.display = "block";
    } finally {
      if (requestId === page.requestId) setPageLoading(false);
    }
  };
  daysSelect.addEventListener("change", update);
  branchSelect.addEventListener("change", update);
}

function aiActivityPage(data) {
  const summary = data.summary;
  const providerRows = (data.top_ai_providers || []).map(
    (item) => `<tr>
    <td>${aiContributorLink(item.key, item.provider, "", item.provider, item.provider)}</td>
    <td class="count">${number.format(item.commits_with_ai)}</td>
    <td class="count">${number.format(item.commits_ai_authored)}</td>
    <td class="count">${number.format(item.commits_ai_coauthored)}</td>
    <td class="count">${number.format(item.commits_ai_assisted)}</td>
    <td class="count">${number.format(item.pull_requests_with_ai)}</td>
    <td class="count">${number.format(item.shared_additions)}</td>
    <td class="count">${number.format(item.shared_deletions)}</td>
  </tr>`,
  );
  const aiRows = data.top_ais.map(
    (item) => `<tr>
    <td>${aiContributorLink(item.provider_key, item.provider, "", item.provider, item.provider)}</td>
    <td>${escapeHtml(item.model || "—")}</td>
    <td class="count">${number.format(item.commits_ai_authored)}</td>
    <td class="count">${number.format(item.commits_ai_coauthored)}</td>
    <td class="count">${number.format(item.commits_ai_assisted)}</td>
    <td class="count">${number.format(item.pull_requests_with_ai)}</td>
    <td class="count">${number.format(item.shared_additions)}</td>
    <td class="count">${number.format(item.shared_deletions)}</td>
  </tr>`,
  );
  const repositoryRows = (data.by_repository || []).map(
    (item) => `<tr>
    <td>${repositoryLink(item.id, item.name, item.target_branch)}</td>
    <td class="count">${number.format(item.commits_ai_authored)}</td>
    <td class="count">${number.format(item.commits_ai_coauthored)}</td>
    <td class="count">${number.format(item.commits_ai_assisted)}</td>
    <td class="count">${number.format(item.pull_requests_ai_authored)}</td>
    <td class="count">${number.format(item.pull_requests_with_ai_authored_commit)}</td>
    <td class="count">${number.format(item.pull_requests_ai_coauthored)}</td>
    <td class="count">${number.format(item.pull_requests_ai_assisted)}</td>
  </tr>`,
  );
  const aiPrRows = data.recent_pull_requests.map(
    (item) => `<tr>
    <td>${formatDate(item.created_at)}</td><td>${externalLink(item.url, `#${item.number} ${item.title}`)}</td>
    <td>${repositoryLink(item.repository_id, item.repository, item.target_branch)}</td><td>${escapeHtml(item.target_branch || "—")}</td><td>${userLink(item.author_id, item.author_name, item.author_login)}</td>
    <td>${escapeHtml(item.categories.map((category) => category.replaceAll("_", " ")).join(", "))}</td>
  </tr>`,
  );
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
    `<p class="notice">${escapeHtml(data.recognition)} Counts can overlap when a commit or PR has more than one AI attribution. Unrecognized trailers are shown as context: ${number.format(summary.commits_with_coauthor_trailer)} commits have a Co-Authored-By trailer and ${number.format(summary.commits_with_assisted_trailer)} have an Assisted-By trailer.</p>
    <section class="panel page-section"><h2>AI providers</h2><div class="subtitle">Provider totals count each commit and pull request once across that provider’s models. Line changes are shared across recognized AI participants.</div>${table(["Provider", "Commits with AI", "AI-authored commits", "AI co-authored commits", "AI-assisted commits", "PRs with AI attribution", "Lines added · shared", "Lines removed · shared"], providerRows, "No recognized AI provider activity in this range.")}</section>
    ${aiModelSharePanel()}
    <section class="panel page-section"><h2>AI model breakdown</h2><div class="subtitle">Model variants remain distinct; context-window annotations are normalized away. Provider-only tools have no model name.</div>${table(["Provider", "Model", "AI-authored commits", "AI co-authored commits", "AI-assisted commits", "PRs with AI attribution", "Lines added · shared", "Lines removed · shared"], aiRows, "No AI models have activity in this range.")}</section>
    <section class="panel page-section"><h2>AI activity by repository</h2>${table(["Repository", "AI-authored commits", "AI co-authored commits", "AI-assisted commits", "AI-authored PRs", "PRs with an AI-authored commit", "PRs with AI co-author", "PRs with AI assistance"], repositoryRows, "No AI attribution has been identified yet.")}</section>
    <section class="page-grid"><article class="panel page-section"><h2>Recent AI-attributed commits</h2>${commitTable(data.recent_commits)}</article><article class="panel page-section"><h2>Recent AI-attributed pull requests</h2>${table(["Created", "Pull request", "Repository", "Target branch", "Author", "AI attribution"], aiPrRows, "No AI-attributed pull requests.")}</article></section>`
  );
}

function aiModelSharePanel() {
  return `<section class="panel page-section" id="ai-model-share">
    <div class="panel-head"><div><h2>AI model share</h2><div class="subtitle" id="ai-model-share-subtitle">Top normalized models by AI-attributed commits</div></div>
      <div class="contributor-toggle" role="group" aria-label="AI model share metric"><button type="button" data-ai-pie-metric="commits_with_ai" aria-pressed="true">Commits</button><button type="button" data-ai-pie-metric="pull_requests_with_ai" aria-pressed="false">Pull requests</button></div>
    </div>
    <div class="project-pie-content"><svg class="project-pie" id="ai-model-pie" viewBox="0 0 128 128" role="img" aria-label="AI model share pie chart"></svg><ul class="pie-legend" id="ai-model-pie-legend"></ul></div>
    <div class="empty" id="ai-model-pie-empty" hidden>No recognized AI model activity in this range.</div>
    <div class="subtitle">A commit or pull request with multiple AI identities contributes to each matching model.</div>
  </section>`;
}

function drawAIModelShare(models, metric) {
  const slices = models
    .map((model, index) => ({
      ...model,
      value: Number(model[metric]) || 0,
      color: chartColor(index),
    }))
    .filter((model) => model.value > 0);
  const total = slices.reduce((sum, model) => sum + model.value, 0);
  const svg = document.getElementById("ai-model-pie");
  const legend = document.getElementById("ai-model-pie-legend");
  const empty = document.getElementById("ai-model-pie-empty");
  const subtitle = document.getElementById("ai-model-share-subtitle");
  subtitle.textContent =
    metric === "commits_with_ai"
      ? "Top normalized models by AI-attributed commits"
      : "Top normalized models by pull requests with AI attribution";
  empty.hidden = total > 0;
  svg.hidden = total === 0;
  legend.hidden = total === 0;
  svg.setAttribute("aria-label", subtitle.textContent);
  if (!total) {
    svg.replaceChildren();
    legend.replaceChildren();
    return;
  }
  const radius = 48;
  const circumference = 2 * Math.PI * radius;
  let offset = 0;
  const circles = slices
    .map((model) => {
      const length = (model.value / total) * circumference;
      const circle = `<circle cx="64" cy="64" r="${radius}" fill="none" style="stroke:${model.color}" stroke-width="24" stroke-dasharray="${length} ${circumference - length}" stroke-dashoffset="${-offset}" transform="rotate(-90 64 64)"><title>${escapeHtml(model.provider)} ${escapeHtml(model.model || "")}: ${number.format(model.value)}</title></circle>`;
      offset += length;
      return circle;
    })
    .join("");
  svg.innerHTML = `${circles}<text x="64" y="61" text-anchor="middle" class="pie-total">${number.format(total)}</text><text x="64" y="77" text-anchor="middle" class="pie-total-label">total</text>`;
  legend.innerHTML = slices
    .map((model) => {
      const label = model.model ? `${model.provider} · ${model.model}` : model.provider;
      return `<li><span class="pie-key" style="--pie-color:${model.color}"></span><span class="pie-project">${aiContributorLink(model.provider_key, model.provider, "", model.provider, label)}</span><span class="pie-value">${number.format(model.value)} · ${Math.round((model.value / total) * 100)}%</span></li>`;
    })
    .join("");
}

function bindAIModelShare(models) {
  let metric = "commits_with_ai";
  const render = () => {
    document.querySelectorAll("[data-ai-pie-metric]").forEach((button) => {
      button.setAttribute("aria-pressed", String(button.dataset.aiPieMetric === metric));
    });
    drawAIModelShare(models, metric);
  };
  document.querySelectorAll("[data-ai-pie-metric]").forEach((button) => {
    button.addEventListener("click", () => {
      metric = button.dataset.aiPieMetric;
      render();
    });
  });
  render();
}

// Render one detail route into #page-content. `requestId` is the navigation's page.requestId;
// a newer navigation makes this render stop before replacing the content.
export async function loadBrowsePage(route, requestId) {
  const { parts, params } = route;
  const section = parts[0] || "";
  const id = parts[1];
  const stale = () => requestId !== page.requestId;
  let data;
  if (section === "repositories" && !id) {
    data = await getJson("/api/repositories");
    if (stale()) return;
    content.innerHTML = repositoryList(data);
    bindRepositorySort(data);
  } else if (section === "repositories") {
    const query = new URLSearchParams(params);
    if (!query.has("days")) query.set("days", "365");
    data = await getJson(`/api/repositories/${encodeURIComponent(id)}?${query.toString()}`);
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
    const query = new URLSearchParams(params);
    if (!query.has("days")) query.set("days", "365");
    data = await getJson(`/api/users/${encodeURIComponent(id)}?${query.toString()}`);
    if (stale()) return;
    currentContributorData = data;
    content.innerHTML = userDetailPage(data);
    bindContributorFilters(id);
    void loadContributorHistory(id, data, page.requestId);
  } else if (section === "companies") {
    const query = new URLSearchParams(params);
    if (!query.has("days")) query.set("days", "365");
    data = await getJson(`/api/companies${id ? `/${encodeURIComponent(id)}` : ""}?${query}`);
    if (stale()) return;
    content.innerHTML = id ? companyDetailPage(data) : companyList(data);
    bindCompanyFilters(data, id);
  } else if (section === "ai-activity") {
    data = await getJson("/api/ai-usage");
    if (stale()) return;
    content.innerHTML = aiActivityPage(data);
    bindAIModelShare(data.top_ais || []);
  } else if (section === "ai-contributors" && id) {
    const query = new URLSearchParams(params);
    if (!query.has("days")) query.set("days", "365");
    data = await getJson(`/api/ai-contributors/${encodeURIComponent(id)}?${query.toString()}`);
    if (stale()) return;
    content.innerHTML = aiContributorDetailPage(data);
    bindAIContributorFilters(id);
    bindAIModelShare(data.models || []);
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
      `${rangeStamp(params.get("days") || "30")} · contributors`;
    if (params.get("section") === "new-contributors") {
      document.getElementById("new-contributors").scrollIntoView();
    }
  } else {
    document.getElementById("page-stamp").textContent = "Last 365 days unless noted";
  }
}
