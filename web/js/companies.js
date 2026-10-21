import { getJson } from "./api.js";
import {
  activityPanel,
  companyLink,
  content,
  contributionPie,
  errorBox,
  escapeHtml,
  formatDate,
  number,
  page,
  pageHead,
  rangeStamp,
  setPageLoading,
  stats,
  table,
  userLink,
} from "./common.js";
import { currentRoute, replaceParams } from "./route.js";

let repositoryCompanyMetric = "pull_requests_merged";

function renderRepositoryCompanyShare(region, data) {
  const query = new URLSearchParams(data.filters).toString();
  const rows = data.companies.map((company) => ({
    ...company,
    name: company.canonical_name,
    query,
  }));
  rows.sort(
    (a, b) =>
      (Number(b[repositoryCompanyMetric]) || 0) - (Number(a[repositoryCompanyMetric]) || 0) ||
      a.name.localeCompare(b.name),
  );
  const controls = `<div class="contributor-toggle" role="group" aria-label="Company contribution metric">${[
    ["pull_requests_merged", "Merged PRs"],
    ["commits", "Commits"],
    ["reviews", "Reviews"],
  ]
    .map(
      ([metric, label]) =>
        `<button type="button" data-repository-company-metric="${metric}" aria-pressed="${metric === repositoryCompanyMetric}">${label}</button>`,
    )
    .join("")}</div>`;
  region.innerHTML = contributionPie("Contributions by company", rows, repositoryCompanyMetric, {
    linkCompanies: true,
    controls,
    subtitle:
      "Shared contribution credits in the selected period and branch. Each contribution counts once for every credited company; Independent is the default when no company affiliation is known.",
  });
  region.querySelectorAll("[data-repository-company-metric]").forEach((button) =>
    button.addEventListener("click", () => {
      repositoryCompanyMetric = button.dataset.repositoryCompanyMetric;
      renderRepositoryCompanyShare(region, data);
    }),
  );
}

export async function loadRepositoryCompanyShare(repositoryId, filters) {
  const region = document.getElementById("repository-company-share");
  const query = new URLSearchParams({
    days: String(filters.days),
    repository_id: repositoryId,
    target_branch: filters.target_branch || "",
  });
  try {
    const data = await getJson(`/api/companies?${query}`);
    // A new repository/period/branch replaces this region. Ignore its old request.
    if (!region.isConnected) return;
    renderRepositoryCompanyShare(region, data);
  } catch (error) {
    if (!region.isConnected) return;
    region.innerHTML = `<section class="panel page-section"><h2>Contributions by company</h2><div class="empty" role="alert">${escapeHtml(`Company contributions could not be loaded: ${error.message}`)}</div></section>`;
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

export function companyProfile(user) {
  const periods = user.affiliation_history || [];
  const rows = periods.map(
    (affiliation) => `<tr>
    <td>${companyLink(affiliation.company_id, affiliation.company_name)}</td>
    <td>${affiliationState(affiliation)}</td>
    <td>${affiliation.valid_from ? formatDate(affiliation.valid_from) : "Unknown"}</td>
    <td>${affiliationEnded(affiliation)}</td>
  </tr>`,
  );
  return `<div class="affiliation-history"><h2>Company affiliations</h2>
    ${table(["Company", "Status", "From", "Affiliation ended"], rows, "No affiliations recorded.")}
    <p class="subtitle">Dates describe the recorded periods used for contribution attribution. Independent is the default when no company affiliation is known.</p></div>`;
}

function companyFilters(data) {
  const filters = data.filters;
  const selected = JSON.stringify([filters.repository_id, filters.target_branch]);
  const days = [
    ["7", "Last 7 days"],
    ["30", "Last 30 days"],
    ["90", "Last 90 days"],
    ["365", "Last 365 days"],
    ["all", "All available data"],
  ];
  return `<section class="filters" aria-label="Company activity filters">
    <div class="field"><label for="company-days">Time range</label><select id="company-days">${days.map(([value, label]) => `<option value="${value}"${value === String(filters.days) ? " selected" : ""}>${label}</option>`).join("")}</select></div>
    <div class="field"><label for="company-repository">Repository</label><select id="company-repository"><option value="">All repositories</option>${data.repository_options
      .map((repo) => {
        const value = JSON.stringify([repo.id, repo.target_branch]);
        return `<option value="${escapeHtml(value)}"${value === selected ? " selected" : ""}>${escapeHtml(repo.name)}</option>`;
      })
      .join("")}</select></div>
    <p class="page-copy">Filters apply to activity. Profile matches include current and former affiliations across all periods.</p>
  </section>`;
}

function companyCoverage(data) {
  return `<p class="notice">Across the selected repositories and period, ${number.format(data.coverage.percent)}% of commits and opened PRs have a curated company mapping or dated period for all credited contributors. Undated mappings apply to all recorded activity unless a dated period overrides them. Activity without a mapping counts as Independent. Shared contributions count once per company and may appear under multiple companies.</p>`;
}

const companyDirectoryColumns = [
  ["canonical_name", "Company"],
  ["profile_matches", "Profile matches"],
  ["commits", "Commits"],
  ["pull_requests_opened", "Opened PRs"],
  ["pull_requests_merged", "Merged PRs"],
  ["shared_additions", "Added · shared"],
  ["shared_deletions", "Removed · shared"],
  ["reviews", "PRs reviewed"],
];
const companyProfileColumns = [
  ["name", "Contributor"],
  ["status", "Status"],
  ["from", "From"],
  ["ended", "Affiliation ended"],
];

function companySortState(params, scope, columns, defaultKey, defaultDirection) {
  const key = params.get(`${scope}_sort`);
  const direction = params.get(`${scope}_dir`);
  return {
    key: columns.some(([candidate]) => candidate === key) ? key : defaultKey,
    direction: ["asc", "desc"].includes(direction) ? direction : defaultDirection,
  };
}

function companySortableTable(columns, rows, emptyText, scope, sort) {
  if (!rows.length) return `<div class="empty">${escapeHtml(emptyText)}</div>`;
  const headers = columns
    .map(
      ([key, label]) =>
        `<th aria-sort="${sort.key === key ? (sort.direction === "asc" ? "ascending" : "descending") : "none"}"><button type="button" class="sort-button" data-company-sort="${key}" data-company-scope="${scope}">${label}${sort.key === key ? (sort.direction === "asc" ? " ↑" : " ↓") : ""}</button></th>`,
    )
    .join("");
  return `<div class="table-wrap"><table><thead><tr>${headers}</tr></thead><tbody>${rows.join("")}</tbody></table></div>`;
}

function companyDirectoryTable(companies, query, sort) {
  const needle = query.trim().toLocaleLowerCase();
  const matching = companies.filter((company) =>
    [company.canonical_name, ...(company.aliases || [])]
      .join(" ")
      .toLocaleLowerCase()
      .includes(needle),
  );
  matching.sort((a, b) => {
    const comparison =
      sort.key === "canonical_name"
        ? String(a[sort.key] || "").localeCompare(String(b[sort.key] || ""))
        : (Number(a[sort.key]) || 0) - (Number(b[sort.key]) || 0);
    return (
      (sort.direction === "asc" ? comparison : -comparison) ||
      a.canonical_name.localeCompare(b.canonical_name)
    );
  });
  const rows = matching.map(
    (company) => `<tr>
    <td class="table-link">${companyLink(company.id, company.canonical_name)}</td>
    <td class="count">${number.format(company.profile_matches)}</td>
    <td class="count">${number.format(company.commits || 0)}</td>
    <td class="count">${number.format(company.pull_requests_opened || 0)}</td>
    <td class="count">${number.format(company.pull_requests_merged || 0)}</td>
    <td class="count">${number.format(company.shared_additions || 0)}</td>
    <td class="count">${number.format(company.shared_deletions || 0)}</td>
    <td class="count">${number.format(company.reviews || 0)}</td>
  </tr>`,
  );
  return companySortableTable(
    companyDirectoryColumns,
    rows,
    "No companies match this search.",
    "company",
    sort,
  );
}

export function companyList(data) {
  const params = currentRoute().params;
  const query = params.get("q") || "";
  const sort = companySortState(
    params,
    "company",
    companyDirectoryColumns,
    "pull_requests_opened",
    "desc",
  );
  return (
    pageHead(
      "Companies",
      "Public profile affiliations and contributions with reviewed employment dates.",
    ) +
    companyFilters(data) +
    companyCoverage(data) +
    `<section class="panel page-section"><div class="search-control"><label for="company-search">Search companies</label><input id="company-search" type="search" placeholder="Company name" value="${escapeHtml(query)}" autocomplete="off"></div>
    <p class="page-copy">Profile matches include current and former affiliations. Contributions use dated affiliations; activity without a known company is counted as Independent.</p>
    <div id="company-results">${companyDirectoryTable(data.companies, query, sort)}</div></section>`
  );
}

function companyProfileMatchesTable(profileMatches, sort) {
  const matches = [...profileMatches];
  const profileValue = (user) =>
    ({
      name: user.name || user.login || "",
      status: affiliationState(user.affiliation),
      from: user.affiliation.valid_from || "",
      ended: user.affiliation.valid_until || "",
    })[sort.key];
  matches.sort((a, b) => {
    const comparison = String(profileValue(a)).localeCompare(String(profileValue(b)));
    return (sort.direction === "asc" ? comparison : -comparison) || a.name.localeCompare(b.name);
  });
  const users = matches.map(
    (user) => `<tr>
    <td>${userLink(user.id, user.name, user.login)}</td>
    <td>${affiliationState(user.affiliation)}</td>
    <td>${user.affiliation.valid_from ? formatDate(user.affiliation.valid_from) : "Unknown"}</td>
    <td>${affiliationEnded(user.affiliation)}</td>
  </tr>`,
  );
  return companySortableTable(
    companyProfileColumns,
    users,
    "No profiles matched.",
    "profile",
    sort,
  );
}

export function companyDetailPage(data) {
  const company = data.company;
  const sort = companySortState(
    currentRoute().params,
    "profile",
    companyProfileColumns,
    "name",
    "asc",
  );
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
        `<section class="page-grid contributor-repository-pies">${contributionPie("Commits by repository", data.repository_activity, "commits")}${contributionPie("Opened PRs by repository", data.repository_activity, "pull_requests_opened")}</section>`
      : `<section class="panel page-section"><div class="empty">No activity attributed to this company in this range. Historical attribution requires reviewed affiliation dates.</div></section>`) +
    `<section class="panel page-section"><h2>Profile matches</h2><p class="page-copy">Current and former contributors across all periods, independent of activity filters. Open a profile for the full affiliation history.</p><div id="company-profile-results">${companyProfileMatchesTable(data.profile_matches, sort)}</div></section>`
  );
}

export function bindCompanyFilters(data, id = "") {
  const search = document.getElementById("company-search");
  const results = document.getElementById(id ? "company-profile-results" : "company-results");
  const scope = id ? "profile" : "company";
  const sort = companySortState(
    currentRoute().params,
    scope,
    id ? companyProfileColumns : companyDirectoryColumns,
    id ? "name" : "pull_requests_opened",
    id ? "asc" : "desc",
  );
  search?.addEventListener("input", () => {
    results.innerHTML = companyDirectoryTable(data.companies, search.value, sort);
    const params = currentRoute().params;
    if (search.value.trim()) params.set("q", search.value.trim());
    else params.delete("q");
    replaceParams(params);
  });
  results.addEventListener("click", (event) => {
    const button = event.target.closest("[data-company-sort]");
    if (!button) return;
    const selected = button.dataset.companySort;
    if (sort.key === selected) sort.direction = sort.direction === "asc" ? "desc" : "asc";
    else {
      sort.key = selected;
      sort.direction = ["canonical_name", "name", "status", "from", "ended"].includes(selected)
        ? "asc"
        : "desc";
    }
    const params = currentRoute().params;
    params.set(`${scope}_sort`, sort.key);
    params.set(`${scope}_dir`, sort.direction);
    replaceParams(params);
    results.innerHTML = id
      ? companyProfileMatchesTable(data.profile_matches, sort)
      : companyDirectoryTable(data.companies, search.value, sort);
  });
  const days = document.getElementById("company-days");
  const repository = document.getElementById("company-repository");
  const update = async () => {
    const requestId = ++page.requestId;
    const params = currentRoute().params;
    params.set("days", days.value);
    const [repo, branch] = repository.value ? JSON.parse(repository.value) : ["", ""];
    for (const [key, value] of [
      ["repository_id", repo],
      ["target_branch", branch],
    ]) {
      if (value) params.set(key, value);
      else params.delete(key);
    }
    setPageLoading(true);
    errorBox.style.display = "none";
    try {
      const next = await getJson(
        `/api/companies${id ? `/${encodeURIComponent(id)}` : ""}?${params}`,
      );
      if (requestId !== page.requestId) return;
      replaceParams(params);
      content.innerHTML = id ? companyDetailPage(next) : companyList(next);
      bindCompanyFilters(next, id);
      document.getElementById("page-stamp").textContent = rangeStamp(String(next.filters.days));
    } catch (error) {
      if (requestId !== page.requestId) return;
      content.innerHTML = id ? companyDetailPage(data) : companyList(data);
      bindCompanyFilters(data, id);
      errorBox.textContent = `Could not update activity; showing previous results. ${error.message}`;
      errorBox.style.display = "block";
    } finally {
      if (requestId === page.requestId) setPageLoading(false);
    }
  };
  days.addEventListener("change", update);
  repository.addEventListener("change", update);
}
