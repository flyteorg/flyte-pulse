import { getJson } from "./api.js";
import {
  companyLink,
  content,
  escapeHtml,
  formatDate,
  number,
  pageHead,
  panel,
  rangeStamp,
  stats,
  userLink,
} from "./common.js";
import { activityPanel, sharePie } from "./components/charts.js";
import {
  bindFilters,
  filterBar,
  rangeField,
  searchField,
  selectField,
  setParam,
  textMatches,
} from "./components/controls.js";
import { METRICS, repositoryShare } from "./components/entities.js";
import { column, dataTable } from "./components/table.js";
import { currentRoute, replaceParams } from "./route.js";

// Contribution credits by company, for the selected activity (`query` keeps it in the links).
function companyShare(id, companies, query, options = {}) {
  return sharePie({
    id,
    title: "Contributions by company",
    rows: companies.map((company) => ({ ...company, name: company.canonical_name })),
    metrics: [
      { ...METRICS.merged, unit: "PR credits", subtitle: "Shared merged PR credits by company" },
      { ...METRICS.commits, unit: "commit credits", subtitle: "Shared commit credits by company" },
      {
        ...METRICS.reviews,
        unit: "review credits",
        subtitle: "Pull request review credits by company",
      },
    ],
    link: (row) => companyLink(row.id, row.name, query),
    neutral: (row) => row.id === "independent",
    empty: "No company activity in this range.",
    ...options,
  });
}

// Fill `region` with company credits for `params` (days, repository_id, target_branch). A
// region replaced in the meantime ignores its response.
export async function loadCompanyShare(region, id, params, options = {}) {
  const query = new URLSearchParams(
    [...params].filter(
      ([key, value]) => value && ["days", "repository_id", "target_branch"].includes(key),
    ),
  ).toString();
  try {
    const data = await getJson(`/api/companies?${query}`);
    if (!region.isConnected) return;
    region.innerHTML = companyShare(id, data.companies, query, options);
  } catch (error) {
    if (!region.isConnected) return;
    region.innerHTML = panel({
      title: "Contributions by company",
      body: `<div class="empty" role="alert">${escapeHtml(`Company contributions could not be loaded: ${error.message}`)}</div>`,
    });
  }
}

function companyWebsite(url, label = "Website ↗") {
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "https:" || parsed.username || parsed.password) return "";
    return `<a class="external-link" href="${escapeHtml(parsed.href)}" target="_blank" rel="noopener noreferrer">${escapeHtml(label)}</a>`;
  } catch {
    return "";
  }
}

function affiliationState(affiliation) {
  if (!affiliation.is_current) return "Former";
  return affiliation.company_id === "independent" && !affiliation.reviewed ? "Default" : "Current";
}

function affiliationEnded(affiliation) {
  if (!affiliation.valid_until) return "—";
  // Stored end dates are exclusive; show the last included calendar day.
  const end = new Date(`${affiliation.valid_until}T00:00:00Z`);
  if (Number.isNaN(end.getTime())) return "—";
  end.setUTCDate(end.getUTCDate() - 1);
  return formatDate(end.toISOString());
}

// Status and period columns for rows whose `affiliation(row)` is an affiliation period.
function affiliationColumns(affiliation) {
  return [
    column.text("status", "Status", { value: (row) => affiliationState(affiliation(row)) }),
    column.date("from", "From", {
      value: (row) => affiliation(row).valid_from,
      cell: (row) =>
        affiliation(row).valid_from ? formatDate(affiliation(row).valid_from) : "Unknown",
    }),
    column.date("ended", "Affiliation ended", {
      value: (row) => affiliation(row).valid_until,
      cell: (row) => affiliationEnded(affiliation(row)),
    }),
  ];
}

export function companyProfile(user) {
  const table = dataTable({
    id: "affiliations",
    columns: [
      column.text("company", "Company", {
        value: (row) => row.company_name,
        cell: (row) => companyLink(row.company_id, row.company_name),
      }),
      ...affiliationColumns((row) => row),
    ],
    rows: user.affiliation_history,
    empty: "No affiliations recorded.",
  });
  return `<div class="affiliation-history"><h2>Company affiliations</h2>${table}
    <p class="subtitle">Dates describe the recorded periods used for contribution attribution. Independent is the default when no company affiliation is known.</p></div>`;
}

function companyFilters(data) {
  const filters = data.filters;
  return filterBar(
    "Company activity filters",
    [
      rangeField("company-days", String(filters.days)),
      selectField({
        id: "company-repository",
        label: "Repository",
        all: "All repositories",
        options: data.repository_options.map((repo) => [
          JSON.stringify([repo.id, repo.target_branch]),
          repo.name,
        ]),
        selected: JSON.stringify([filters.repository_id, filters.target_branch]),
      }),
    ],
    '<p class="page-copy">Filters apply to activity. Profile matches include current and former affiliations across all periods.</p>',
  );
}

function companyCoverage(data) {
  return `<p class="notice">Across the selected repositories and period, ${number.format(data.coverage.percent)}% of commits and opened PRs have a curated company mapping or dated period for all credited contributors. Undated mappings apply to all recorded activity unless a dated period overrides them. Activity without a mapping counts as Independent. Shared contributions count once per company and may appear under multiple companies.</p>`;
}

function companyTable(companies, query) {
  return dataTable({
    id: "companies",
    columns: [
      column.text("name", "Company", {
        value: (row) => row.canonical_name,
        cell: (row) => companyLink(row.id, row.canonical_name),
        className: "table-link",
      }),
      column.count("profile_matches", "Profile matches"),
      column.count("commits", "Commits"),
      column.count("pull_requests_opened", "Opened PRs"),
      column.count("pull_requests_merged", "Merged PRs"),
      column.count("shared_additions", "Added · shared"),
      column.count("shared_deletions", "Removed · shared"),
      column.count("reviews", "PRs reviewed"),
    ],
    rows: companies.filter((company) =>
      textMatches(query, [company.canonical_name, ...(company.aliases || [])]),
    ),
    empty: "No companies match this search.",
    sort: { key: "pull_requests_opened", direction: "desc" },
  });
}

export function companyList(data) {
  const query = currentRoute().params.get("q") || "";
  return (
    pageHead(
      "Companies",
      "Public profile affiliations and contributions with reviewed employment dates.",
    ) +
    companyFilters(data) +
    companyCoverage(data) +
    panel({
      title: "All companies",
      body: `${searchField({ id: "company-search", label: "Search companies", placeholder: "Company name", value: query })}
    <p class="page-copy">Profile matches include current and former affiliations. Contributions use dated affiliations; activity without a known company is counted as Independent.</p>
    <div id="company-results">${companyTable(data.companies, query)}</div>`,
    })
  );
}

function profileMatchesTable(profileMatches) {
  return dataTable({
    id: "profile-matches",
    columns: [
      column.text("name", "Contributor", {
        value: (row) => (row.name || row.login || "").toLocaleLowerCase(),
        cell: (row) => userLink(row.id, row.name, row.login),
      }),
      ...affiliationColumns((row) => row.affiliation),
    ],
    rows: profileMatches,
    empty: "No profiles matched.",
    sort: { key: "name", direction: "asc" },
  });
}

export function companyDetailPage(data) {
  const company = data.company;
  return (
    pageHead(company.canonical_name, "Community contributions and public profile affiliations.") +
    (company.website_url
      ? `<p class="page-copy">${companyWebsite(company.website_url, "Visit company website ↗")}</p>`
      : "") +
    companyFilters(data) +
    companyCoverage(data) +
    stats(data.summary, [
      ["Commits", "commits"],
      ["Opened PRs", "pull_requests_opened"],
      ["Merged PRs", "pull_requests_merged"],
      ["Lines added · shared", "shared_additions"],
      ["Lines removed · shared", "shared_deletions"],
      ["PRs reviewed", "reviews"],
    ]) +
    (data.daily.length
      ? activityPanel(data.daily) +
        repositoryShare(
          "company-repositories",
          data.repository_activity,
          [METRICS.commits, METRICS.opened, METRICS.merged, METRICS.reviews],
          { subtitle: "Credited activity by repository in the selected range" },
        )
      : panel({
          title: "Activity",
          body: '<div class="empty">No activity attributed to this company in this range. Historical attribution requires reviewed affiliation dates.</div>',
        })) +
    panel({
      title: "Profile matches",
      body: `<p class="page-copy">Current and former contributors across all periods, independent of activity filters. Open a profile for the full affiliation history.</p>${profileMatchesTable(data.profile_matches)}`,
    })
  );
}

export function bindCompanyFilters(data, id = "") {
  const search = document.getElementById("company-search");
  search?.addEventListener("input", () => {
    document.getElementById("company-results").innerHTML = companyTable(
      data.companies,
      search.value,
    );
    const params = currentRoute().params;
    setParam(params, "q", search.value.trim());
    replaceParams(params);
  });
  const days = document.getElementById("company-days");
  const repository = document.getElementById("company-repository");
  bindFilters([days, repository], {
    apply: (params) => {
      params.set("days", days.value);
      const [repo, branch] = repository.value ? JSON.parse(repository.value) : ["", ""];
      setParam(params, "repository_id", repo);
      setParam(params, "target_branch", branch);
    },
    url: (params) => `/api/companies${id ? `/${encodeURIComponent(id)}` : ""}?${params}`,
    render: (next) => {
      content.innerHTML = id ? companyDetailPage(next) : companyList(next);
      bindCompanyFilters(next, id);
      document.getElementById("page-stamp").textContent = rangeStamp(String(next.filters.days));
    },
  });
}
