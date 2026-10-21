import { getJson, snapshotMeta } from "./api.js";
import {
  errorBox,
  escapeHtml,
  formatStamp,
  number,
  page,
  panel,
  pendingPanel,
  reviewPanel,
  stats,
} from "./common.js";
import { loadCompanyShare } from "./companies.js";
import { activityPanel, languagePanel } from "./components/charts.js";
import {
  bindFilters,
  filterBar,
  rangeField,
  rangeParam,
  searchField,
  selectField,
  setParam,
  textMatches,
  toggle,
} from "./components/controls.js";
import {
  METRICS,
  aiModelColumns,
  contributorColumns,
  repositoryColumn,
  repositoryShare,
  reviewerColumns,
} from "./components/entities.js";
import { column, dataTable } from "./components/table.js";
import { PulseLoading } from "./loading.js";
import { currentRoute, replaceParams } from "./route.js";

const KINDS = {
  HUMAN: {
    label: "Human",
    title: "Top human contributors",
    subtitle: "Line changes are shared across the PR author and recognized co-authors",
    rows: "human_top_contributors",
    columns: () => contributorColumns(),
    noun: "contributors",
    empty: "No human contributor activity in this range.",
  },
  NEW: {
    label: "New",
    title: "New contributors",
    subtitle: "First recorded contribution in the selected range and repository, newest first",
    rows: "new_top_contributors",
    columns: () => contributorColumns({ first: true }),
    noun: "new contributors",
    empty: "No new contributors in this range.",
  },
  AI: {
    label: "AI",
    title: "Top AI contributors",
    subtitle: "AI line changes are shared across each recognized AI identity",
    rows: "ai_top_contributors",
    columns: aiModelColumns,
    noun: "AI identities",
    empty: "No recognized AI attribution in this range.",
  },
  REVIEWERS: {
    label: "Reviewers",
    title: "Top reviewers",
    subtitle:
      "Pull requests reviewed in the selected range; each reviewer counts once per pull request",
    rows: "top_reviewers",
    columns: reviewerColumns,
    noun: "reviewers",
    empty: "No reviews in this range.",
  },
};

function summaryCards(summary, newContributorsHref, pending = false) {
  return stats(
    summary,
    [
      [
        "Active human contributors",
        "active_human_contributors",
        undefined,
        { note: "GitHub identities classified as human" },
      ],
      [
        "New human contributors",
        "new_human_contributors",
        undefined,
        { note: "first recorded activity in selected range", href: newContributorsHref },
      ],
      [
        "Pull requests merged",
        "pull_requests_merged",
        undefined,
        { note: "merged in selected range" },
      ],
      ["Commits", "commits", undefined, { note: "associated with tracked pull requests" }],
      [
        "Pull requests opened",
        "pull_requests_opened",
        undefined,
        { note: "opened in selected range" },
      ],
    ],
    { pending, className: "summary-cards" },
  );
}

// Render the overview into `root` and return a cleanup function for navigation away.
export function mountOverview(root) {
  const params = currentRoute().params;
  let kind = "HUMAN";
  let query = params.get("q") || "";
  let currentData = null;
  let active = true;
  const $ = (id) => document.getElementById(id);

  root.innerHTML =
    filterBar(
      "Dashboard filters",
      [
        rangeField("overview-days", rangeParam(params, "30")),
        selectField({
          id: "overview-repository",
          label: "Repository",
          all: "All repositories",
          options: [],
          className: "repo",
        }),
      ],
      '<div class="spacer"></div><div class="status" id="overview-status"></div>',
    ) +
    `<div id="overview-body">${summaryCards({}, "", true)}${pendingPanel("Activity over time")}</div>`;
  const days = $("overview-days");
  const repository = $("overview-repository");

  const dashboardUrl = (values) => {
    const query = new URLSearchParams({ days: rangeParam(values, "30") });
    if (values.get("repository_id")) query.set("repository_id", values.get("repository_id"));
    return `/api/dashboard?${query}`;
  };

  function contributorResults(data) {
    const view = KINDS[kind];
    if (kind === "AI" && data.ai_status !== "loaded") {
      return data.ai_status === "error"
        ? `<div class="empty" role="alert">${escapeHtml(data.ai_error)}</div>`
        : `<div class="loading-state">${PulseLoading.indicator("Loading AI contributor activity…")}</div>`;
    }
    const rows = (data[view.rows] || []).filter((row) =>
      textMatches(query, [row.login || "", row.name || "", row.provider || "", row.model || ""]),
    );
    $("overview-contributor-status").textContent = query.trim()
      ? `${number.format(rows.length)} match${rows.length === 1 ? "" : "es"}`
      : `${number.format(rows.length)} ${view.noun}`;
    return dataTable({
      id: `overview-${kind.toLowerCase()}`,
      columns: view.columns(),
      rows,
      empty: query.trim() ? `No ${view.noun} match “${query.trim()}”.` : view.empty,
    });
  }

  function contributorsPanel() {
    const view = KINDS[kind];
    const ai = kind === "AI";
    return panel({
      title: view.title,
      subtitle: view.subtitle,
      controls: toggle({
        label: "Contributor list",
        options: Object.entries(KINDS).map(([key, item]) => [key, item.label]),
        selected: kind,
      }),
      className: "contributors-panel",
      id: "overview-contributors",
      body: `${searchField({
        id: "overview-contributor-search",
        label: ai ? "Search by model name" : "Search by username or name",
        placeholder: ai ? "Model name" : "Username or full name",
        value: query,
        statusId: "overview-contributor-status",
      })}<div id="overview-contributor-results"></div>`,
    });
  }

  function showContributors() {
    const results = $("overview-contributor-results");
    $("overview-contributor-status").textContent = "";
    results.innerHTML = contributorResults(currentData);
    if (kind === "AI") void loadAIContributors(currentData);
  }

  async function loadAIContributors(data) {
    if (data.ai_status) return;
    data.ai_status = "loading";
    try {
      const result = await getJson(`/api/ai-dashboard?${data.ai_params}`);
      data.ai_top_contributors = result.ai_top_contributors || [];
      data.ai_status = "loaded";
    } catch (error) {
      data.ai_status = "error";
      data.ai_error = `AI contributor activity could not be loaded: ${error.message}`;
    }
    if (active && currentData === data && kind === "AI") showContributors();
  }

  function overviewBody(data) {
    const s = data.summary;
    const newContributors = new URLSearchParams({ days: days.value, section: "new-contributors" });
    setParam(newContributors, "repository_id", repository.value);
    return (
      summaryCards(s, `#/users?${newContributors}`) +
      `<section class="grid overview-grid">${activityPanel(data.daily, {
        id: "overview-activity",
        subtitle: "Daily merged PRs and commits across selected repositories, UTC",
      })}${contributorsPanel()}<div class="overview-shares">${repositoryShare(
        "overview-repositories",
        data.repository_activity,
        [
          { ...METRICS.merged, subtitle: "Merged pull requests by repository" },
          { ...METRICS.commits, subtitle: "Commits by repository" },
          { ...METRICS.opened, subtitle: "Opened pull requests by repository" },
        ],
        { large: true, empty: "No repository activity in this range." },
      )}<div id="overview-company-share">${pendingPanel("Contributions by company")}</div></div></section>` +
      panel({
        title: "Repository activity",
        subtitle:
          "Merged PR line changes counted once per repository; contributor counts are shared and overlap.",
        body: dataTable({
          id: "repository-activity",
          columns: [
            repositoryColumn(),
            column.count("pull_requests_merged", "Merged PRs"),
            column.count("commits", "Commits"),
            column.count("pull_requests_opened", "Opened PRs"),
            column.count("additions", "Lines added"),
            column.count("deletions", "Lines removed"),
          ],
          rows: data.repository_activity,
          empty: "No repository activity in this range.",
        }),
      }) +
      reviewPanel(s, "Merged pull requests in the selected range and repositories") +
      languagePanel(
        "Languages",
        data.languages,
        "Current code in the selected repositories, from GitHub’s language detection",
      )
    );
  }

  let filtersBound = false;
  function show(data) {
    const selection = new URLSearchParams(dashboardUrl(currentRoute().params).split("?")[1]);
    data.ai_params = selection.toString();
    currentData = data;
    if (repository.options.length === 1) {
      for (const item of data.repositories) repository.add(new Option(item.name, item.id));
      repository.value = selection.get("repository_id") || "";
    }
    $("overview-body").innerHTML = overviewBody(data);
    showContributors();
    void loadCompanyShare($("overview-company-share"), "overview-companies", selection, {
      large: true,
      note: "Each contribution counts once for every credited company. Undated mappings cover all recorded activity unless a dated period overrides them. Without a known affiliation, contributions count as Independent.",
    });
    $("page-stamp").textContent = `Metrics updated ${formatStamp(data.metrics_generated_at)}`;
    $("artifact-info").textContent =
      `Dataset ${data.serving_version || data.metrics_version || snapshotMeta().version || "unknown"} · ${number.format(data.summary.active_repositories)} active repositories`;
    $("overview-status").textContent = `${data.repositories.length} repositories`;
    if (!filtersBound) {
      filtersBound = true;
      bindFilters([days, repository], {
        apply: (values) => {
          values.set("days", days.value);
          setParam(values, "repository_id", repository.value);
        },
        url: dashboardUrl,
        render: show,
      });
    }
  }

  root.addEventListener("click", onClick);
  root.addEventListener("input", onInput);
  function onClick(event) {
    const button = event.target.closest("#overview-contributors [data-toggle-value]");
    if (!button || !currentData) return;
    kind = button.dataset.toggleValue;
    $("overview-contributors").outerHTML = contributorsPanel();
    showContributors();
    $("overview-contributors").querySelector(`[data-toggle-value="${kind}"]`).focus();
  }
  function onInput(event) {
    if (event.target.id !== "overview-contributor-search" || !currentData) return;
    query = event.target.value;
    const values = currentRoute().params;
    setParam(values, "q", query.trim());
    replaceParams(values);
    showContributors();
  }

  const requestId = page.requestId;
  $("overview-status").innerHTML = PulseLoading.indicator("Loading activity…", true);
  getJson(dashboardUrl(params))
    .then((data) => {
      if (active && requestId === page.requestId) show(data);
    })
    .catch((error) => {
      if (!active || requestId !== page.requestId) return;
      errorBox.textContent = error.message;
      errorBox.style.display = "block";
      $("overview-status").textContent = "Activity unavailable";
      $("overview-body").innerHTML = panel({
        title: "Activity",
        body: '<div class="empty">Activity could not be loaded.</div>',
      });
      $("page-stamp").textContent = "Activity unavailable";
      $("artifact-info").textContent = "Activity could not be loaded";
    });

  return () => {
    // Stop in-flight requests from writing into the next page.
    active = false;
    root.removeEventListener("click", onClick);
    root.removeEventListener("input", onInput);
  };
}
