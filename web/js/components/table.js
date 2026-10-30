import { escapeHtml, formatDate, formatPercent, number } from "../common.js";
import { currentRoute, replaceParams } from "../route.js";
import { textMatches } from "./controls.js";

// Every table sorts by any column, and columns can filter their rows. Clicking a heading
// re-sorts the table in place and keeps the order in the URL (`<id>_sort`, `<id>_dir`);
// a filter selection is kept as `<id>_<key>`, so a page only describes its columns.
//
// A column is { key, label, title?, value?, cell?, type?, sortable?, className?, filter?,
// filterValue? }: `title` explains a short label, `value(row)` is what it sorts by (default
// `row[key]`), `cell(row)` its HTML (default the escaped value), and `type` "number" or
// "date" right-aligns numbers and sorts largest or newest first. `filter` "choice" adds a
// dropdown of the column's distinct values above the table, "text" a search box;
// `filterValue(row)` (default `value`) is the text or list of texts a row matches with.
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

// The table's filters: columns with `filter`, then the spec's own `filters`, each as
// { key, label, kind, value(row) }.
function filterDefs(spec) {
  const fromColumns = spec.columns
    .filter((item) => item.filter)
    .map((item) => ({
      key: item.key,
      label: item.label,
      kind: item.filter,
      value: item.filterValue || ((row) => valueOf(item, row)),
    }));
  return [...fromColumns, ...(spec.filters || []).map((f) => ({ kind: "choice", ...f }))];
}

const texts = (value) =>
  (Array.isArray(value) ? value : [value])
    .filter((text) => text != null && text !== "")
    .map(String);

// The selected value of each filter, from the URL.
function filterState(spec) {
  const params = currentRoute().params;
  const state = new Map();
  for (const def of filterDefs(spec)) {
    const selected = params.get(`${spec.id}_${def.key}`);
    if (selected) state.set(def.key, selected);
  }
  return state;
}

function filtered(spec, state) {
  if (!state.size) return spec.rows;
  const defs = filterDefs(spec).filter((def) => state.has(def.key));
  return spec.rows.filter((row) =>
    defs.every((def) => {
      const values = texts(def.value(row));
      const selected = state.get(def.key);
      return def.kind === "text" ? textMatches(selected, values) : values.includes(selected);
    }),
  );
}

function filterControl(spec, def, selected) {
  const id = `${spec.id}-filter-${def.key}`;
  const label = `<label for="${id}">${escapeHtml(def.label)}</label>`;
  if (def.kind === "text") {
    return `<div class="field">${label}<input id="${id}" type="search" data-filter-key="${escapeHtml(def.key)}" value="${escapeHtml(selected)}" autocomplete="off"></div>`;
  }
  const counts = new Map();
  for (const row of spec.rows) {
    for (const text of new Set(texts(def.value(row))))
      counts.set(text, (counts.get(text) || 0) + 1);
  }
  const options = [...counts.keys()]
    .sort((a, b) => a.localeCompare(b))
    .map(
      (text) =>
        `<option value="${escapeHtml(text)}"${text === selected ? " selected" : ""}>${escapeHtml(text)} (${number.format(counts.get(text))})</option>`,
    )
    .join("");
  return `<div class="field">${label}<select id="${id}" data-filter-key="${escapeHtml(def.key)}"><option value="">All</option>${options}</select></div>`;
}

function filterBar(spec, state) {
  const defs = filterDefs(spec);
  if (!defs.length) return "";
  const controls = defs.map((def) => filterControl(spec, def, state.get(def.key) || "")).join("");
  return `<div class="table-filters" role="group" aria-label="Table filters">${controls}${filterStatus(spec, state)}</div>`;
}

function filterStatus(spec, state) {
  const shown = filtered(spec, state).length;
  const total = spec.rows.length;
  const clear = state.size
    ? ` <button type="button" class="link-button" data-filter-clear>Clear</button>`
    : "";
  return `<div class="table-filter-status" aria-live="polite">${number.format(shown)} of ${number.format(total)} rows${clear}</div>`;
}

function renderRows(spec) {
  const rows = filtered(spec, filterState(spec));
  if (!rows.length) return `<div class="empty">No rows match the filters.</div>`;
  const sort = sortState(spec);
  const headings = spec.columns.map((item) => heading(item, sort)).join("");
  const body = ordered({ ...spec, rows }, sort)
    .map((row) => `<tr>${spec.columns.map((item) => cell(item, row)).join("")}</tr>`)
    .join("");
  // Long lists scroll after about ten rows with the header kept in view.
  return `<div class="table-wrap table-scroll"><table><thead><tr>${headings}</tr></thead><tbody>${body}</tbody></table></div>`;
}

function render(spec) {
  if (!spec.rows.length) return `<div class="empty">${escapeHtml(spec.empty)}</div>`;
  return `${filterBar(spec, filterState(spec))}<div class="table-rows">${renderRows(spec)}</div>`;
}

// Redraw the rows and the match count after a sort or filter change, keeping the controls.
function refresh(region, spec) {
  const rows = region.querySelector(".table-rows");
  if (rows) rows.innerHTML = renderRows(spec);
  const status = region.querySelector(".table-filter-status");
  if (status) status.outerHTML = filterStatus(spec, filterState(spec));
}

// `id` names the table in the URL and must be unique on its page. `sort` is the initial
// { key, direction }; without one, rows keep their given order. `filters` adds filters that
// are not columns, as { key, label, value(row) } choices.
export function dataTable({ id, columns, rows, empty = "No data available.", sort, filters }) {
  const spec = { id, columns, rows: rows || [], empty, sort, filters };
  tables.set(id, spec);
  return `<div class="data-table" data-table="${escapeHtml(id)}">${render(spec)}</div>`;
}

function tableOf(element) {
  const region = element?.closest("[data-table]");
  const spec = region && tables.get(region.dataset.table);
  return spec ? { region, spec } : null;
}

function setFilter(spec, key, value) {
  const params = currentRoute().params;
  if (value) params.set(`${spec.id}_${key}`, value);
  else params.delete(`${spec.id}_${key}`);
  replaceParams(params);
}

document.addEventListener("change", (event) => {
  const select = event.target.closest("select[data-filter-key]");
  const table = tableOf(select);
  if (!table) return;
  setFilter(table.spec, select.dataset.filterKey, select.value);
  refresh(table.region, table.spec);
});

document.addEventListener("input", (event) => {
  const input = event.target.closest("input[data-filter-key]");
  const table = tableOf(input);
  if (!table) return;
  setFilter(table.spec, input.dataset.filterKey, input.value.trim());
  refresh(table.region, table.spec);
});

document.addEventListener("click", (event) => {
  const clear = event.target.closest("[data-filter-clear]");
  const cleared = tableOf(clear);
  if (cleared) {
    const params = currentRoute().params;
    for (const def of filterDefs(cleared.spec)) params.delete(`${cleared.spec.id}_${def.key}`);
    replaceParams(params);
    cleared.region.innerHTML = render(cleared.spec);
    return;
  }
  const button = event.target.closest("[data-sort-key]");
  const table = tableOf(button);
  if (!table) return;
  const { region, spec } = table;
  const key = button.dataset.sortKey;
  const current = sortState(spec);
  const item = spec.columns.find((candidate) => candidate.key === key);
  const direction =
    current?.key === key ? (current.direction === "asc" ? "desc" : "asc") : firstDirection(item);
  const params = currentRoute().params;
  params.set(`${spec.id}_sort`, key);
  params.set(`${spec.id}_dir`, direction);
  replaceParams(params);
  refresh(region, spec);
  region.querySelector(`[data-sort-key="${CSS.escape(key)}"]`)?.focus();
});
