import { escapeHtml, formatDate, formatPercent, number } from "../common.js";
import { currentRoute, replaceParams } from "../route.js";

// Every table sorts by any column. Clicking a heading re-sorts the table in place and keeps
// the order in the URL (`<id>_sort`, `<id>_dir`), so a page only describes its columns.
//
// A column is { key, label, title?, value?, cell?, type?, sortable?, className? }: `title`
// explains a short label, `value(row)` is what it sorts by (default `row[key]`), `cell(row)`
// its HTML (default the escaped value), and `type` "number" or "date" right-aligns numbers
// and sorts largest or newest first.
const tables = new Map();

export const column = {
  text: (key, label, options = {}) => ({ key, label, ...options }),
  count: (key, label, options = {}) => ({
    key,
    label,
    type: "number",
    value: (row) => Number(row[key]) || 0,
    cell: (row) => number.format(Number(row[key]) || 0),
    ...options,
  }),
  // Fractions such as review coverage; rows without a value sort last.
  percent: (key, label, options = {}) => ({
    key,
    label,
    type: "number",
    cell: (row) => formatPercent(row[key]),
    ...options,
  }),
  date: (key, label, options = {}) => {
    const value = options.value || ((row) => row[key]);
    return { key, label, type: "date", value, cell: (row) => formatDate(value(row)), ...options };
  },
};

function valueOf(item, row) {
  return item.value ? item.value(row) : row[item.key];
}

function firstDirection(item) {
  return item.type === "number" || item.type === "date" ? "desc" : "asc";
}

function sortState(spec) {
  const params = currentRoute().params;
  const key = params.get(`${spec.id}_sort`);
  const item = spec.columns.find((candidate) => candidate.key === key);
  if (item && item.sortable !== false) {
    const direction = params.get(`${spec.id}_dir`);
    return {
      key,
      direction: ["asc", "desc"].includes(direction) ? direction : firstDirection(item),
    };
  }
  return spec.sort || null;
}

function compare(a, b) {
  return typeof a === "number" && typeof b === "number"
    ? a - b
    : String(a).localeCompare(String(b));
}

// Rows keep their given order until a column is chosen. Missing values sort last, and ties
// fall back to the first column.
function ordered(spec, sort) {
  const item = sort && spec.columns.find((candidate) => candidate.key === sort.key);
  if (!item) return spec.rows;
  const sign = sort.direction === "asc" ? 1 : -1;
  const missing = (value) => value == null || value === "";
  const first = spec.columns[0];
  return [...spec.rows].sort((x, y) => {
    const a = valueOf(item, x);
    const b = valueOf(item, y);
    const order = missing(a) || missing(b) ? missing(a) - missing(b) : sign * compare(a, b);
    return order || compare(valueOf(first, x) ?? "", valueOf(first, y) ?? "");
  });
}

function heading(item, sort) {
  const numeric = item.type === "number" ? ' class="num"' : "";
  const label = escapeHtml(item.label);
  const title = item.title ? ` title="${escapeHtml(item.title)}"` : "";
  if (item.sortable === false) return `<th${numeric}${title}>${label}</th>`;
  const active = sort?.key === item.key;
  const ascending = sort?.direction === "asc";
  const state = active ? (ascending ? "ascending" : "descending") : "none";
  return `<th${numeric}${title} aria-sort="${state}"><button type="button" class="sort-button" data-sort-key="${escapeHtml(item.key)}">${label}<span aria-hidden="true">${active ? (ascending ? " ↑" : " ↓") : ""}</span></button></th>`;
}

function cell(item, row) {
  const classes = [item.type === "number" ? "count" : "", item.className || ""]
    .filter(Boolean)
    .join(" ");
  const value = item.cell ? "" : valueOf(item, row);
  const html = item.cell ? item.cell(row) : escapeHtml(value == null || value === "" ? "—" : value);
  return `<td${classes ? ` class="${classes}"` : ""}>${html}</td>`;
}

function render(spec) {
  if (!spec.rows.length) return `<div class="empty">${escapeHtml(spec.empty)}</div>`;
  const sort = sortState(spec);
  const headings = spec.columns.map((item) => heading(item, sort)).join("");
  const rows = ordered(spec, sort)
    .map((row) => `<tr>${spec.columns.map((item) => cell(item, row)).join("")}</tr>`)
    .join("");
  // Long lists scroll after about ten rows with the header kept in view.
  return `<div class="table-wrap table-scroll"><table><thead><tr>${headings}</tr></thead><tbody>${rows}</tbody></table></div>`;
}

// `id` names the table in the URL and must be unique on its page. `sort` is the initial
// { key, direction }; without one, rows keep their given order.
export function dataTable({ id, columns, rows, empty = "No data available.", sort }) {
  const spec = { id, columns, rows: rows || [], empty, sort };
  tables.set(id, spec);
  return `<div class="data-table" data-table="${escapeHtml(id)}">${render(spec)}</div>`;
}

document.addEventListener("click", (event) => {
  const button = event.target.closest("[data-sort-key]");
  const region = button?.closest("[data-table]");
  const spec = region && tables.get(region.dataset.table);
  if (!spec) return;
  const key = button.dataset.sortKey;
  const current = sortState(spec);
  const item = spec.columns.find((candidate) => candidate.key === key);
  const direction =
    current?.key === key ? (current.direction === "asc" ? "desc" : "asc") : firstDirection(item);
  const params = currentRoute().params;
  params.set(`${spec.id}_sort`, key);
  params.set(`${spec.id}_dir`, direction);
  replaceParams(params);
  region.innerHTML = render(spec);
  region.querySelector(`[data-sort-key="${CSS.escape(key)}"]`)?.focus();
});
