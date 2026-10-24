// Journey: one contributor's history from first contribution to latest activity, with
// milestones and achievements, and a landing page to find a contributor.

import {
  encodePath,
  escapeHtml,
  externalLink,
  link,
  number,
  pageHead,
  panel,
  stats,
} from "./common.js";
import { activityPanel } from "./components/charts.js";
import { searchField, setParam, textMatches } from "./components/controls.js";
import { achievementGrid, timeline } from "./components/timeline.js";
import { currentRoute, replaceParams } from "./route.js";

const MAX_MATCHES = 25;

export function journeyHref(id) {
  return `#/users/${encodePath(id)}/journey`;
}

function matches(users, query) {
  const needle = query.trim();
  if (!needle) return [];
  return users
    .filter((user) => textMatches(needle, [user.name || "", user.login || ""]))
    .sort((a, b) => (b.pull_requests_merged || 0) - (a.pull_requests_merged || 0))
    .slice(0, MAX_MATCHES);
}

function matchList(users, query) {
  const rows = matches(users, query);
  if (!query.trim())
    return `<p class="subtitle">Type a name or GitHub login to find a journey.</p>`;
  if (!rows.length)
    return `<div class="empty">No contributor matches “${escapeHtml(query.trim())}”.</div>`;
  return `<ul class="journey-matches">${rows
    .map(
      (user) =>
        `<li><a class="data-link" href="${journeyHref(user.id)}">${escapeHtml(user.name || user.login)}</a>${user.login && user.name ? `<span class="user-alias">@${escapeHtml(user.login)}</span>` : ""}<span class="journey-match-note">${number.format(user.pull_requests_merged || 0)} merged PRs</span></li>`,
    )
    .join("")}</ul>`;
}

// `data` is the contributor directory (`/api/users`).
export function journeyLanding(data, params) {
  const query = params.get("q") || "";
  return (
    pageHead(
      "Journey",
      "One contributor's path through Flyte: first contribution, milestones, achievements, and activity over the years.",
    ) +
    panel({
      title: "Find a contributor",
      body:
        searchField({
          id: "journey-search",
          label: "Contributor",
          placeholder: "Name or GitHub login",
          value: query,
        }) + `<div id="journey-matches">${matchList(data.users || [], query)}</div>`,
    })
  );
}

export function bindJourneySearch(data) {
  const input = document.getElementById("journey-search");
  input.addEventListener("input", () => {
    document.getElementById("journey-matches").innerHTML = matchList(data.users || [], input.value);
    const params = currentRoute().params;
    setParam(params, "q", input.value.trim());
    replaceParams(params);
  });
  input.focus();
}

function tenure(span) {
  if (!span.first_date) return "No recorded contributions yet";
  const parts = [`Contributing since ${span.first_date}`];
  if (span.years > 0) parts.push(`${span.years} ${span.years === 1 ? "year" : "years"}`);
  if (span.last_date && span.last_date !== span.first_date)
    parts.push(`last active ${span.last_date}`);
  return parts.join(" · ");
}

function affiliationStrip(history) {
  if (!history?.length) return "";
  const items = history
    .slice()
    .reverse()
    .map((row) => {
      const period = row.valid_from
        ? `${row.valid_from} – ${row.valid_until || "now"}`
        : row.valid_until
          ? `until ${row.valid_until}`
          : "all recorded time";
      const name =
        row.company_id === "independent"
          ? escapeHtml(row.company_name)
          : link(`/companies/${encodeURIComponent(row.company_id)}`, row.company_name);
      return `<li${row.is_current ? ' class="current"' : ""}>${name}<small>${escapeHtml(period)}</small></li>`;
    })
    .join("");
  return `<ul class="affiliation-strip" aria-label="Affiliations">${items}</ul>`;
}

// A tier label ("top 5%", "#1") only for the ranks worth showing; other cards stay plain.
function rankNote(ranks, key) {
  const rank = ranks?.[key];
  return rank ? `${rank.tier} of ${number.format(rank.of)} contributors` : "";
}

function highlights(data) {
  const busiest = data.highlights?.busiest_month;
  const streak = data.highlights?.longest_streak;
  const lines = [];
  if (busiest) {
    lines.push(
      `Busiest month: ${busiest.month}, with ${number.format(busiest.pull_requests_merged)} merged PRs, ${number.format(busiest.commits)} commits, and ${number.format(busiest.reviews)} reviews.`,
    );
  }
  if (streak) {
    lines.push(
      streak.months === 1
        ? "Longest run of consecutive active months: 1."
        : `Longest run of consecutive active months: ${number.format(streak.months)}, from ${streak.from} to ${streak.to}.`,
    );
  }
  return lines.length ? `<p class="page-copy">${lines.map(escapeHtml).join(" ")}</p>` : "";
}

export function journeyPage(data) {
  const user = data.contributor;
  const profile = user.profile_url
    ? externalLink(user.profile_url, `@${user.login || user.name}`)
    : escapeHtml(user.login || user.id);
  const totals = { ...data.totals, active_months: data.span.active_months };
  const unlocked = (data.achievements || []).filter((badge) => badge.unlocked).length;
  return (
    pageHead(`${user.name}'s journey`, tenure(data.span)) +
    `<section class="panel page-section journey-intro"><div class="page-copy">${profile} · ${link(`/users/${user.id}`, "Contributor page")}</div>${affiliationStrip(user.affiliation_history)}</section>` +
    stats(
      totals,
      [
        [
          "Merged PRs",
          "pull_requests_merged",
          number.format,
          { note: rankNote(data.ranks, "pull_requests_merged") },
        ],
        ["Commits", "commits", number.format, { note: rankNote(data.ranks, "commits") }],
        ["PRs reviewed", "reviews", number.format, { note: rankNote(data.ranks, "reviews") }],
        [
          "Lines changed · shared",
          "shared_lines",
          number.format,
          { note: rankNote(data.ranks, "shared_lines") },
        ],
        ["Repositories", "repositories"],
        ["Active months", "active_months"],
      ],
      { className: "journey-cards" },
    ) +
    activityPanel(data.monthly, {
      id: "journey-activity",
      title: "Activity by month",
      subtitle: "Monthly merged PRs and commits across all repositories, UTC",
      empty: "No activity recorded.",
    }) +
    highlights(data) +
    `<section class="page-grid journey-grid">${panel({
      title: "Timeline",
      subtitle: `${number.format((data.timeline || []).length)} moments, oldest first`,
      body: timeline(data.timeline),
      className: "journey-timeline",
    })}${panel({
      title: "Achievements",
      subtitle: `${unlocked} of ${(data.achievements || []).length} unlocked`,
      body: achievementGrid(data.achievements),
      className: "journey-achievements",
    })}</section>`
  );
}
