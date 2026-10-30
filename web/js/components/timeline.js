// A dated, vertical list of events, oldest first, and a grid of achievement badges.

import { escapeHtml, externalLink } from "../common.js";
import { currentRoute, replaceParams, routeHref } from "../route.js";
import { badgeIcon, linkIcon } from "./icons.js";

const KIND_LABELS = {
  first: "First",
  milestone: "Milestone",
  repository: "Repository",
  affiliation: "Affiliation",
  anniversary: "Anniversary",
  record: "Record",
  ai: "AI",
  latest: "Latest",
};

function eventTitle(event) {
  const title = escapeHtml(event.title);
  if (!event.url) return title;
  if (event.url.startsWith("#/"))
    return `<a class="data-link" href="${escapeHtml(event.url)}">${title}</a>`;
  return externalLink(event.url, event.title);
}

// `events` are { date, kind, title, detail?, url? }.
export function timeline(events, { empty = "No recorded activity yet." } = {}) {
  if (!events?.length) return `<div class="empty">${escapeHtml(empty)}</div>`;
  const items = events
    .map(
      (event) =>
        `<li class="timeline-item kind-${escapeHtml(event.kind)}"><time datetime="${escapeHtml(event.date)}">${escapeHtml(event.date)}</time><div class="timeline-body"><span class="timeline-kind">${escapeHtml(KIND_LABELS[event.kind] || event.kind)}</span><div class="timeline-title">${eventTitle(event)}</div>${event.detail ? `<div class="timeline-detail">${escapeHtml(event.detail)}</div>` : ""}</div></li>`,
    )
    .join("");
  return `<ol class="timeline">${items}</ol>`;
}

// `achievements` are { key, name, description, unlocked, unlocked_at?, value?, target? }.
// The badge whose key is `highlight` is marked, and each badge has a button that copies a
// link to the page with that badge highlighted (`?badge=<key>`).
export function achievementGrid(achievements, { highlight = "" } = {}) {
  if (!achievements?.length) return `<div class="empty">No achievements defined.</div>`;
  const badges = achievements
    .map((badge) => {
      const progress =
        !badge.unlocked && badge.target != null
          ? `<div class="badge-progress" role="progressbar" aria-valuemin="0" aria-valuemax="${badge.target}" aria-valuenow="${badge.value || 0}"><span style="width:${Math.min(100, Math.round(((badge.value || 0) / badge.target) * 100))}%"></span></div><div class="badge-note">${escapeHtml(String(badge.value || 0))} of ${escapeHtml(String(badge.target))}</div>`
          : "";
      const when = badge.unlocked
        ? `<div class="badge-note">Unlocked ${escapeHtml(badge.unlocked_at || "")}</div>`
        : "";
      const marked = badge.key === highlight ? " highlighted" : "";
      const share = `<button type="button" class="badge-share" data-badge-link="${escapeHtml(badge.key)}" aria-label="Copy a link to ${escapeHtml(badge.name)}" title="Copy link">${linkIcon()}</button><span class="badge-share-note" aria-live="polite"></span>`;
      return `<li class="badge${badge.unlocked ? " unlocked" : " locked"}${marked}" id="badge-${escapeHtml(badge.key)}" data-key="${escapeHtml(badge.key)}"><div class="badge-icon">${badgeIcon(badge.key)}</div><div class="badge-text"><div class="badge-name">${escapeHtml(badge.name)}</div><div class="badge-description">${escapeHtml(badge.description)}</div>${when}${progress}</div>${share}</li>`;
    })
    .join("");
  return `<ul class="badge-grid">${badges}</ul>`;
}

// Scroll the highlighted badge into view after the page is in the document.
export function revealBadge(key) {
  if (!key) return;
  document.getElementById(`badge-${key}`)?.scrollIntoView({ block: "center" });
}

async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

// Copy the link to a badge, highlight it, and put the badge in the page address.
document.addEventListener("click", async (event) => {
  const button = event.target.closest("[data-badge-link]");
  if (!button) return;
  const key = button.dataset.badgeLink;
  const route = currentRoute();
  route.params.set("badge", key);
  replaceParams(route.params);
  for (const badge of document.querySelectorAll(".badge.highlighted")) {
    badge.classList.remove("highlighted");
  }
  button.closest(".badge")?.classList.add("highlighted");
  const url = new URL(routeHref(route.path, route.params), location.href).href;
  const copied = await copyText(url);
  const note = button.nextElementSibling;
  if (note?.classList.contains("badge-share-note")) {
    note.textContent = copied ? "Link copied" : "Link is in the address bar";
    window.setTimeout(() => {
      if (note.isConnected) note.textContent = "";
    }, 2500);
  }
});
