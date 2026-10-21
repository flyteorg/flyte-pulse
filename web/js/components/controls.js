import { getJson } from "../api.js";
import { errorBox, escapeHtml, page, setPageLoading } from "../common.js";
import { currentRoute, replaceParams } from "../route.js";

export const RANGES = [
  ["7", "Last 7 days"],
  ["30", "Last 30 days"],
  ["90", "Last 90 days"],
  ["365", "Last 365 days"],
  ["all", "All available data"],
];

// The time range from the URL, or `fallback` when it is missing or unknown.
export function rangeParam(params, fallback) {
  const days = params.get("days");
  return RANGES.some(([value]) => value === days) ? days : fallback;
}

// `options` are [value, label] pairs; `all` adds a first option with an empty value.
export function selectField({ id, label, options, selected = "", all = "", className = "" }) {
  const choices = (all ? [["", all], ...options] : options)
    .map(
      ([value, text]) =>
        `<option value="${escapeHtml(value)}"${String(value) === String(selected) ? " selected" : ""}>${escapeHtml(text)}</option>`,
    )
    .join("");
  return `<div class="field${className ? ` ${className}` : ""}"><label for="${id}">${escapeHtml(label)}</label><select id="${id}">${choices}</select></div>`;
}

export function rangeField(id, selected) {
  return selectField({ id, label: "Time range", options: RANGES, selected });
}

export function filterBar(label, fields, extra = "") {
  return `<section class="filters" aria-label="${escapeHtml(label)}">${fields.join("")}${extra}</section>`;
}

// A search box; `statusId` adds a live line under it for match counts.
export function searchField({ id, label, placeholder, value = "", hint = "", statusId = "" }) {
  const status = statusId ? `<div class="subtitle" id="${statusId}" aria-live="polite"></div>` : "";
  return `<div class="search-control"><label for="${id}">${escapeHtml(label)}</label><input id="${id}" type="search" value="${escapeHtml(value)}" placeholder="${escapeHtml(placeholder)}" autocomplete="off">${hint ? `<div class="subtitle">${escapeHtml(hint)}</div>` : ""}${status}</div>`;
}

// A row of buttons that selects one of `options` ([value, label] pairs). Each button carries
// `data-toggle-value`; the owner of the toggle handles clicks.
export function toggle({ label, options, selected }) {
  return `<div class="toggle" role="group" aria-label="${escapeHtml(label)}">${options
    .map(
      ([value, text]) =>
        `<button type="button" data-toggle-value="${escapeHtml(value)}" aria-pressed="${value === selected}">${escapeHtml(text)}</button>`,
    )
    .join("")}</div>`;
}

export function setParam(params, key, value) {
  if (value) params.set(key, value);
  else params.delete(key);
}

export function textMatches(query, values) {
  const needle = query.trim().toLocaleLowerCase();
  return !needle || values.join(" ").toLocaleLowerCase().includes(needle);
}

// Query the page again when one of `selects` changes. `apply` writes the selection into the
// URL parameters and `render(data, requestId)` shows the result of `url(params)`. A newer
// selection or navigation drops older responses; a failed query restores the previous
// selection and keeps its results.
export function bindFilters(selects, { apply, url, render, failed }) {
  let previous = selects.map((select) => select.value);
  const update = async () => {
    const requestId = ++page.requestId;
    const params = currentRoute().params;
    apply(params);
    replaceParams(params);
    setPageLoading(true);
    errorBox.style.display = "none";
    try {
      const data = await getJson(url(params));
      if (requestId !== page.requestId) return;
      previous = selects.map((select) => select.value);
      render(data, requestId);
    } catch (error) {
      if (requestId !== page.requestId) return;
      selects.forEach((select, index) => (select.value = previous[index]));
      const restored = currentRoute().params;
      apply(restored);
      replaceParams(restored);
      errorBox.textContent = `Could not update activity; showing previous results. ${error.message}`;
      errorBox.style.display = "block";
      failed?.(requestId);
    } finally {
      if (requestId === page.requestId) setPageLoading(false);
    }
  };
  for (const select of selects) select.addEventListener("change", update);
}
