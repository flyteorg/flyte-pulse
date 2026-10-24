// A dated, vertical list of events, oldest first, and a grid of achievement badges.

import { escapeHtml, externalLink } from "../common.js";

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
export function achievementGrid(achievements) {
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
      return `<li class="badge${badge.unlocked ? " unlocked" : " locked"}" data-key="${escapeHtml(badge.key)}"><div class="badge-name">${escapeHtml(badge.name)}</div><div class="badge-description">${escapeHtml(badge.description)}</div>${when}${progress}</li>`;
    })
    .join("");
  return `<ul class="badge-grid">${badges}</ul>`;
}
