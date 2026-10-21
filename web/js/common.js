import { PulseLoading } from "./loading.js";

export const content = document.getElementById("page-content");
export const errorBox = document.getElementById("error");
export const number = new Intl.NumberFormat();
// Incremented by every page navigation and filter change; stale responses compare against it.
export const page = { requestId: 0 };

export function setPageLoading(busy) {
  content.setAttribute("aria-busy", String(busy));
  const status = document.getElementById("page-loading-status");
  if (busy) status.innerHTML = PulseLoading.indicator("Updating activity…", true);
  status.hidden = !busy;
  content.querySelectorAll(".panel, .card").forEach((section) => {
    section.classList.toggle("is-refreshing", busy && section.getAttribute("aria-busy") !== "true");
  });
}

export function pendingPanel(title, failed = false) {
  const body = failed
    ? '<div class="empty">This section could not be loaded.</div>'
    : `<div class="loading-state">${PulseLoading.indicator("Loading details…")}</div>`;
  return panel({ title, body, attributes: `aria-busy="${!failed}"` });
}

export function escapeHtml(value) {
  return String(value ?? "").replace(
    /[&<>"']/g,
    (char) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#39;",
      })[char],
  );
}

export function externalLink(url, label) {
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "https:" || parsed.hostname !== "github.com") {
      return escapeHtml(label);
    }
    return `<a class="external-link" href="${escapeHtml(parsed.href)}" target="_blank" rel="noopener noreferrer">${escapeHtml(label)}</a>`;
  } catch {
    return escapeHtml(label);
  }
}

export function formatDate(value) {
  if (!value) return "—";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return escapeHtml(value);
  return escapeHtml(parsed.toLocaleDateString(undefined, { dateStyle: "medium", timeZone: "UTC" }));
}

export function formatStamp(value) {
  if (!value) return "not available";
  const dt = new Date(value);
  return Number.isNaN(dt.getTime())
    ? value
    : dt.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short", timeZone: "UTC" }) +
        " UTC";
}

// Internal links are hash routes; `path` is the route path, e.g. /repositories/<id>.
export function link(path, label, details = "") {
  return `<a class="data-link" href="${escapeHtml(`#${path}`)}">${escapeHtml(label)}</a>${details ? `<small>${escapeHtml(details)}</small>` : ""}`;
}

export function pageHead(title, description = "") {
  return `<header class="page-head"><h1>${escapeHtml(title)}</h1>${description ? `<p>${escapeHtml(description)}</p>` : ""}</header>`;
}

// A titled section. `subtitle` and `controls` (such as a toggle or legend) go in its head.
export function panel({
  title,
  subtitle = "",
  controls = "",
  body,
  className = "",
  id = "",
  attributes = "",
}) {
  const heading = escapeHtml(title);
  const head =
    subtitle || controls
      ? `<div class="panel-head"><div><h2>${heading}</h2>${subtitle ? `<div class="subtitle">${escapeHtml(subtitle)}</div>` : ""}</div>${controls}</div>`
      : `<h2>${heading}</h2>`;
  const classes = `panel page-section${className ? ` ${className.trim()}` : ""}`;
  return `<section class="${classes}"${id ? ` id="${id}"` : ""}${attributes ? ` ${attributes}` : ""}>${head}${body}</section>`;
}

// `entries` are [label, key, format?, { note?, href? }]; `format` is for values that are
// not counts, `href` links the value. With `pending`, missing values show as loading.
export function stats(summary, entries, { pending = false, className = "" } = {}) {
  return `<section class="cards${className ? ` ${className}` : ""}" aria-label="Summary">${entries
    .map(([label, key, format = number.format, { note = "", href = "" } = {}]) => {
      const loading = pending && summary[key] == null;
      let value = loading
        ? PulseLoading.indicator("Loading…", true)
        : summary[key] == null
          ? "—"
          : escapeHtml(format(summary[key]));
      if (href && !loading)
        value = `<a class="summary-link" href="${escapeHtml(href)}">${value}</a>`;
      return `<article class="card" aria-busy="${loading}"><div class="card-label">${escapeHtml(label)}</div><div class="card-value">${value}</div>${note ? `<div class="card-note">${escapeHtml(note)}</div>` : ""}</article>`;
    })
    .join("")}</section>`;
}

export function repositoryLink(id, name, targetBranch = "") {
  const query = targetBranch ? `?target_branch=${encodeURIComponent(targetBranch)}` : "";
  return link(`/repositories/${encodeURIComponent(id)}${query}`, name || id);
}

export function userLink(id, name, login) {
  const label = escapeHtml(name || login || id || "Unknown");
  const alias = login && name ? `<span class="user-alias">@${escapeHtml(login)}</span>` : "";
  if (!id) return `${label}${alias}`;
  return `<a class="data-link" href="#/users/${encodeURIComponent(id)}">${label}</a>${alias}`;
}

export function companyLink(id, name, query = "") {
  return link(`/companies/${encodeURIComponent(id)}${query ? `?${query}` : ""}`, name || id);
}

export function aiContributorLink(key, provider, model, name, label = "") {
  if (!key) return escapeHtml(model || provider || name || "Unknown AI");
  const query = new URLSearchParams({
    provider: provider || "",
    model: model || "",
    name: name || "",
  });
  return link(
    `/ai-contributors/${encodeURIComponent(key)}?${query.toString()}`,
    label || model || provider || name || "Unknown AI",
  );
}

export function aiIdentityLinks(identities) {
  return identities.map((identity) => userLink(identity.id, identity.name)).join(", ");
}

// A fraction such as 0.874 as "87%".
export function formatPercent(share) {
  return share == null ? "—" : `${Math.round(share * 100)}%`;
}

export function formatHours(hours) {
  if (hours == null) return "—";
  if (hours < 1) return `${Math.max(1, Math.round(hours * 60))} min`;
  if (hours < 48) return `${hours.toFixed(1)} h`;
  return `${(hours / 24).toFixed(1)} days`;
}

// Review coverage and response time for the merged pull requests in a dashboard summary.
export function reviewPanel(summary, subtitle) {
  const coverage = summary.merged_pull_requests_for_coverage
    ? `${number.format(summary.merged_pull_requests_reviewed)} of ${number.format(summary.merged_pull_requests_for_coverage)} merged pull requests had a review by someone other than the author before they merged.`
    : "No pull requests were merged in this range.";
  return panel({
    title: "Code review",
    subtitle,
    className: "review-panel",
    body: `${stats(summary, [
      ["Review coverage", "review_coverage", formatPercent],
      ["Median time to first review", "median_hours_to_first_review", formatHours],
      ["PRs reviewed", "reviews"],
      ["Active reviewers", "active_reviewers"],
    ])}<p class="page-copy">${escapeHtml(coverage)} Time to first review runs from opening, including time as a draft. Each reviewer counts once per pull request; reviews of one’s own pull requests and reviews by bots and AI accounts are not counted.</p>`,
  });
}

export function rangeStamp(days) {
  return days === "all" ? "All available data" : `Last ${days} days`;
}
