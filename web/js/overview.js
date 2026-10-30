import { getJson, snapshotMeta } from "./api.js";
import {
  aiContributorLink,
  errorBox,
  escapeHtml,
  formatDate,
  formatStamp,
  highlights,
  number,
  page,
  panel,
  pendingPanel,
  reviewPanel,
  stats,
  userLink,
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
  targetBranchFilter,
} from "./components/entities.js";
import { column, dataTable } from "./components/table.js";
import { PulseLoading } from "./loading.js";
import { currentRoute, replaceParams } from "./route.js";

// The list order matches the highlight cards: contribution views from broadest to
// narrowest, then reviews, then AI.
const KINDS = {
  ALL: {
    label: "All",
    title: "Top contributors",
    subtitle: "Line changes are shared across the PR author and recognized co-authors",
    rows: "human_top_contributors",
    sort: "pull_requests_merged",
    columns: () => contributorColumns(),
    noun: "contributors",
    empty: "No contributor activity in this range.",
  },
  COMMUNITY: {
    label: "Community",
    title: "Top community contributors",
    subtitle: (data) =>
      `${communityScope(data)}; line changes are shared across the PR author and co-authors`,
    rows: "community_top_contributors",
    sort: "pull_requests_merged",
    columns: () => contributorColumns(),
    noun: "community contributors",
    empty: "No community contributions in this range.",
  },
  NEW: {
    label: "New",
    title: "New contributors",
    subtitle: "First recorded contribution in the selected range and repository",
    rows: "new_top_contributors",
    sort: "pull_requests_merged",
    columns: () => contributorColumns({ first: true }),
    noun: "new contributors",
    empty: "No new contributors in this range.",
  },
  REVIEWERS: {
    label: "Reviewers",
    title: "Top reviewers",
    subtitle:
      "Pull requests reviewed in the selected range; each reviewer counts once per pull request",
    rows: "top_reviewers",
    sort: "reviews",
    columns: reviewerColumns,
    noun: "reviewers",
    empty: "No reviews in this range.",
  },
  AI: {
    label: "AI",
    title: "Top AI contributors",
    subtitle: "AI line changes are shared across each recognized AI identity",
    rows: "ai_top_contributors",
    sort: "commits_with_ai",
    columns: aiModelColumns,
    noun: "AI identities",
    empty: "No recognized AI attribution in this range.",
  },
};

// Community activity is what people did while not at a maintainer company.
function maintainers(data) {
  const names = data.maintainer_companies || [];
  return new Intl.ListFormat("en", { type: "disjunction" }).format(names);
}

function communityScope(data) {
  return data.maintainer_companies?.length
    ? `Activity while not affiliated with ${maintainers(data)}`
    : "Every contributor; no maintainer company is configured";
}

// The first row of `rows` sorted by `key` like a table column: largest first, ties by name.
function leader(rows, key, name = (row) => (row.name || row.login || "").toLocaleLowerCase()) {
  return (rows || [])
    .filter((row) => Number(row[key]) > 0)
    .sort((a, b) => b[key] - a[key] || name(a).localeCompare(name(b)))[0];
}

function count(value, singular, plural) {
  return `${number.format(value)} ${value === 1 ? singular : plural}`;
}

// Leaders of the contributor lists, each linking to its full ranking.
function highlightCards(data) {
  const person = (row) => row && userLink(row.id, row.name, row.login);
  // Each card shows the first row of its list in that list's default order.
  const first = (kind, name) => leader(data[KINDS[kind].rows], KINDS[kind].sort, name);
  const ranking = (kind) =>
    `<button type="button" class="link-button" data-ranking="${kind}">See ranking</button>`;
  const merged = (row, none) =>
    row ? count(row.pull_requests_merged, "merged PR", "merged PRs") : none;
  const top = first("ALL");
  const community = first("COMMUNITY");
  const reviewer = first("REVIEWERS");
  const newcomer = first("NEW");
  const model = data.ai_status === "loaded" && first("AI", (row) => row.provider || "");
  const modelName = model && (model.model ? `${model.provider} · ${model.model}` : model.provider);
  return highlights([
    {
      label: "Top contributor",
      subject: person(top),
      value: merged(top, "No merged PRs in this range"),
      action: ranking("ALL"),
    },
    {
      label: "Top community contributor",
      subject: person(community),
      value: merged(community, "No community merged PRs in this range"),
      note: data.maintainer_companies?.length ? `Outside ${maintainers(data)}` : "",
      action: ranking("COMMUNITY"),
    },
    {
      label: "Top new contributor",
      subject: person(newcomer),
      value: newcomer
        ? `${merged(newcomer)} · first on ${formatDate(newcomer.first_contribution_date)}`
        : "No merged PRs from new contributors",
      action: ranking("NEW"),
    },
    {
      label: "Top reviewer",
      subject: person(reviewer),
      value: reviewer
        ? count(reviewer.reviews, "PR reviewed", "PRs reviewed")
        : "No reviews in this range",
      action: ranking("REVIEWERS"),
    },
    {
      label: "Most used AI model",
      subject: model
        ? aiContributorLink(model.provider_key, model.provider, "", model.provider, modelName)
        : data.ai_status === "error"
          ? "—"
          : PulseLoading.indicator("Loading…", true),
      value: model
        ? count(model.commits_with_ai, "AI-attributed commit", "AI-attributed commits")
        : data.ai_status === "loaded"
          ? "No AI attribution in this range"
          : "",
      action: ranking("AI"),
    },
  ]);
}

function summaryCards(summary, newContributorsHref, pending = false) {
  return stats(
    summary,
    [
      [
        "Active contributors",
        "active_human_contributors",
        undefined,
        { note: "GitHub identities classified as human" },
      ],
      [
        "New contributors",
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
  let kind = "ALL";
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
    if (values.get("repository")) query.set("repository", values.get("repository"));
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
      sort: { key: view.sort, direction: "desc" },
    });
  }

  function contributorsPanel() {
    const view = KINDS[kind];
    const ai = kind === "AI";
    return panel({
      title: view.title,
      subtitle: typeof view.subtitle === "function" ? view.subtitle(currentData) : view.subtitle,
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
      })}<div class="contributors-results" id="overview-contributor-results"></div>`,
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
    if (!active || currentData !== data) return;
    $("overview-highlights").innerHTML = highlightCards(data);
    if (kind === "AI") showContributors();
  }

  function overviewBody(data) {
    const s = data.summary;
    const newContributors = new URLSearchParams({ days: days.value, section: "new-contributors" });
    setParam(newContributors, "repository", repository.value);
    return (
      `<div id="overview-highlights">${highlightCards(data)}</div>` +
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
          filters: [targetBranchFilter()],
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
      repository.value = selection.get("repository") || "";
    }
    $("overview-body").innerHTML = overviewBody(data);
    showContributors();
    // The AI ranking is a separate query; the highlights show its leader.
    void loadAIContributors(data);
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
          setParam(values, "repository", repository.value);
        },
        url: dashboardUrl,
        render: show,
      });
    }
  }

  root.addEventListener("click", onClick);
  root.addEventListener("input", onInput);
  function showKind(next) {
    kind = next;
    $("overview-contributors").outerHTML = contributorsPanel();
    showContributors();
  }
  function onClick(event) {
    if (!currentData) return;
    const ranking = event.target.closest("[data-ranking]");
    if (ranking) {
      // Open the leader's list in its default order, the card's metric.
      const table = `overview-${ranking.dataset.ranking.toLowerCase()}`;
      const values = currentRoute().params;
      values.delete(`${table}_sort`);
      values.delete(`${table}_dir`);
      replaceParams(values);
      showKind(ranking.dataset.ranking);
      $("overview-contributors").scrollIntoView({ behavior: "smooth", block: "start" });
      $("overview-contributors")
        .querySelector("[aria-pressed='true']")
        .focus({ preventScroll: true });
      return;
    }
    const button = event.target.closest("#overview-contributors [data-toggle-value]");
    if (!button) return;
    showKind(button.dataset.toggleValue);
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
