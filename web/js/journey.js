// Journey: one contributor's history from first contribution to latest activity, with
// milestones and achievements. It is the second tab of the contributor page.

import { encodePath, number, pageHead, panel } from "./common.js";
import { achievementGrid, timeline } from "./components/timeline.js";

// The tab bar under a contributor's page title; `active` is "activity" or "journey".
export function contributorTabs(id, active) {
  const base = `#/users/${encodePath(id)}`;
  const tab = (href, key, label) =>
    `<a href="${href}"${active === key ? ' aria-current="page"' : ""}>${label}</a>`;
  return `<nav class="page-tabs" aria-label="Contributor views">${tab(base, "activity", "Activity")}${tab(`${base}/journey`, "journey", "Journey")}</nav>`;
}

function tenure(span) {
  if (!span.first_date) return "No recorded contributions yet";
  const parts = [`Contributing since ${span.first_date}`];
  if (span.years > 0) parts.push(`${span.years} ${span.years === 1 ? "year" : "years"}`);
  if (span.last_date && span.last_date !== span.first_date)
    parts.push(`last active ${span.last_date}`);
  return parts.join(" · ");
}

// `badge` is the key of the achievement to highlight, from the page's `badge` parameter.
export function journeyPage(data, { badge = "" } = {}) {
  const user = data.contributor;
  const unlocked = (data.achievements || []).filter((badge) => badge.unlocked).length;
  return (
    pageHead(user.name, tenure(data.span)) +
    contributorTabs(user.id, "journey") +
    `<section class="page-grid journey-grid">${panel({
      title: "Timeline",
      subtitle: `${number.format((data.timeline || []).length)} moments, oldest first`,
      body: timeline(data.timeline),
      className: "journey-timeline",
    })}${panel({
      title: "Achievements",
      subtitle: `${unlocked} of ${(data.achievements || []).length} unlocked`,
      body: achievementGrid(data.achievements, { highlight: badge }),
      className: "journey-achievements",
    })}</section>`
  );
}
