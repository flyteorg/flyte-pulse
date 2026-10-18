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
  return `<section class="panel page-section" aria-busy="${!failed}"><h2>${escapeHtml(title)}</h2>${body}</section>`;
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

export function stats(summary, entries, pending = false) {
  return `<section class="cards" aria-label="Summary">${entries
    .map(([label, key]) => {
      const loading = pending && summary[key] == null;
      const value = loading
        ? PulseLoading.indicator("Loading…", true)
        : summary[key] == null
          ? "—"
          : number.format(summary[key]);
      return `<article class="card" aria-busy="${loading}"><div class="card-label">${escapeHtml(label)}</div><div class="card-value">${value}</div></article>`;
    })
    .join("")}</section>`;
}

export function activityPanel(points, options = {}) {
  if (!points?.length) return "";
  const firstKey = options.firstKey || "pull_requests_merged";
  const secondKey = options.secondKey || "commits";
  const firstLabel = options.firstLabel || "Merged PRs";
  const secondLabel = options.secondLabel || "Commits";
  const title = options.title || "Activity over time";
  const subtitle = options.subtitle || "Daily merged PRs and commits, UTC";
  const width = 760,
    height = 220;
  const pad = { left: 38, right: 38, top: 12, bottom: 28 };
  const plotWidth = width - pad.left - pad.right;
  const plotHeight = height - pad.top - pad.bottom;
  const maxValue = Math.max(
    1,
    ...points.map((point) => Math.max(Number(point[secondKey]) || 0, Number(point[firstKey]) || 0)),
  );
  const scaleMax =
    Math.ceil(maxValue / (maxValue < 10 ? 2 : maxValue < 40 ? 5 : 10)) *
    (maxValue < 10 ? 2 : maxValue < 40 ? 5 : 10);
  const x = (index) =>
    pad.left + (points.length <= 1 ? plotWidth / 2 : (index * plotWidth) / (points.length - 1));
  const y = (value) => pad.top + plotHeight - (value / scaleMax) * plotHeight;
  const path = (key) =>
    points
      .map((point, index) => `${index ? "L" : "M"}${x(index)},${y(Number(point[key]) || 0)}`)
      .join(" ");
  const grid = Array.from({ length: 5 }, (_, index) => {
    const value = Math.round((scaleMax * (4 - index)) / 4);
    const lineY = pad.top + (plotHeight * index) / 4;
    return `<line x1="${pad.left}" y1="${lineY}" x2="${width - pad.right}" y2="${lineY}" class="gridline"/><text x="${pad.left - 8}" y="${lineY + 3}" text-anchor="end" class="axis-label">${number.format(value)}</text>`;
  }).join("");
  const labelCount = Math.min(points.length, 7);
  const labelIndices = new Set(
    Array.from({ length: labelCount }, (_, index) =>
      labelCount === 1 ? 0 : Math.round((index * (points.length - 1)) / (labelCount - 1)),
    ),
  );
  const labels = points
    .map((point, index) =>
      labelIndices.has(index)
        ? `<text x="${x(index)}" y="${height - 7}" text-anchor="middle" class="axis-label">${escapeHtml(String(point.date || ""))}</text>`
        : "",
    )
    .join("");
  return `<article class="panel page-section"><div class="panel-head"><div><h2>${escapeHtml(title)}</h2><div class="subtitle">${escapeHtml(subtitle)}</div></div><div class="legend"><span><i class="dot"></i>${escapeHtml(firstLabel)}</span><span><i class="dot orange"></i>${escapeHtml(secondLabel)}</span></div></div><svg class="chart" viewBox="0 0 ${width} ${height}" role="img" aria-label="${escapeHtml(subtitle)}">${grid}<path d="${path(firstKey)}" class="series-a"/><path d="${path(secondKey)}" class="series-b"/>${labels}</svg></article>`;
}

export function table(headers, rows, emptyText = "No data available.") {
  if (!rows.length) return `<div class="empty">${escapeHtml(emptyText)}</div>`;
  return `<div class="table-wrap"><table><thead><tr>${headers.map((header) => `<th>${header}</th>`).join("")}</tr></thead><tbody>${rows.join("")}</tbody></table></div>`;
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

export function contributionPie(
  title,
  rows,
  metric,
  { linkRepositories = true, linkCompanies = false, subtitle = "", controls = "" } = {},
) {
  const palette = [
    "#4bd4df",
    "#728bff",
    "#ffb86b",
    "#68d9a2",
    "#d78bff",
    "#f17c9b",
    "#a9c66b",
    "#65a9ff",
  ];
  const slices = rows
    .map((row, index) => ({
      ...row,
      value: Number(row[metric]) || 0,
      color:
        linkCompanies && row.id === "independent" ? "#8193a9" : palette[index % palette.length],
    }))
    .filter((row) => row.value > 0);
  const total = slices.reduce((sum, row) => sum + row.value, 0);
  const heading = `<div class="panel-head"><div><h2>${escapeHtml(title)}</h2><div class="subtitle">${number.format(total)} total · selected time range</div>${subtitle ? `<p class="page-copy">${escapeHtml(subtitle)}</p>` : ""}</div>${controls}</div>`;
  if (!total) {
    return `<article class="panel page-section">${heading}<div class="empty">No activity in this time range.</div></article>`;
  }
  const radius = 48;
  const circumference = 2 * Math.PI * radius;
  let offset = 0;
  const circles = slices
    .map((row) => {
      const length = (row.value / total) * circumference;
      const circle = `<circle cx="64" cy="64" r="${radius}" fill="none" stroke="${row.color}" stroke-width="24" stroke-dasharray="${length} ${circumference - length}" stroke-dashoffset="${-offset}" transform="rotate(-90 64 64)"><title>${escapeHtml(row.name)}: ${number.format(row.value)}</title></circle>`;
      offset += length;
      return circle;
    })
    .join("");
  const legend = slices
    .map((row) => {
      const label = linkCompanies
        ? companyLink(row.id, row.name, row.query)
        : linkRepositories
          ? repositoryLink(row.id, row.name, row.target_branch)
          : escapeHtml(row.name);
      return `<li><span class="pie-key" style="--pie-color:${row.color}"></span><span class="pie-project">${label}</span><span class="pie-value">${number.format(row.value)} · ${Math.round((row.value / total) * 100)}%</span></li>`;
    })
    .join("");
  return `<article class="panel page-section project-pie-panel">
    ${heading}
    <div class="project-pie-content"><svg class="project-pie" width="112" height="112" style="width:112px;height:112px;max-width:112px;max-height:112px;display:block;flex:none" viewBox="0 0 128 128" role="img" aria-label="${escapeHtml(title)} breakdown">${circles}<text x="64" y="61" text-anchor="middle" fill="#edf4fb" style="font:700 17px system-ui">${number.format(total)}</text><text x="64" y="77" text-anchor="middle" fill="#91a3b9" style="font:10px system-ui">total</text></svg><ul class="pie-legend">${legend}</ul></div>
  </article>`;
}

export function rangeStamp(days) {
  return days === "all" ? "All available data" : `Last ${days} days`;
}
