import { escapeHtml, number, panel } from "../common.js";
import { toggle } from "./controls.js";

// Chart colors are CSS variables (dashboard.css), so charts follow the light/dark theme.
const CHART_COLOR_COUNT = 8;
export const NEUTRAL_CHART_COLOR = "var(--chart-neutral)";

export function chartColor(index) {
  return `var(--chart-${(index % CHART_COLOR_COUNT) + 1})`;
}

/* Activity over time */

const SERIES = [
  { key: "pull_requests_merged", label: "Merged PRs" },
  { key: "commits", label: "Commits" },
];
const charts = new Map();

// Two daily series as lines. The SVG is drawn at its rendered width, and redrawn when that
// changes, so labels keep their size on narrow screens.
function activitySvg(spec, width) {
  const { points, series } = spec;
  const height = 230;
  const pad = { left: 36, right: 40, top: 15, bottom: 32 };
  const plotWidth = width - pad.left - pad.right;
  const plotHeight = height - pad.top - pad.bottom;
  const value = (point, key) => Number(point[key]) || 0;
  const maximum = Math.max(1, ...points.flatMap((point) => series.map((s) => value(point, s.key))));
  const step = maximum < 10 ? 2 : maximum < 40 ? 5 : 10;
  const scaleMax = Math.max(step, Math.ceil(maximum / step) * step);
  const x = (index) =>
    pad.left + (points.length <= 1 ? plotWidth / 2 : (index * plotWidth) / (points.length - 1));
  const y = (amount) => pad.top + plotHeight - (amount / scaleMax) * plotHeight;
  const grid = Array.from({ length: 5 }, (_, index) => {
    const lineY = pad.top + (plotHeight * index) / 4;
    const label = number.format(Math.round((scaleMax * (4 - index)) / 4));
    return `<line x1="${pad.left}" y1="${lineY}" x2="${width - pad.right}" y2="${lineY}" class="gridline"/><text x="${pad.left - 8}" y="${lineY + 3}" text-anchor="end" class="axis-label">${label}</text>`;
  }).join("");
  const lines = series
    .map((s, index) => {
      const suffix = index ? "b" : "a";
      const path = points
        .map((point, i) => `${i ? "L" : "M"}${x(i)},${y(value(point, s.key))}`)
        .join(" ");
      const dots = points
        .map((point, i) =>
          value(point, s.key) > 0
            ? `<circle cx="${x(i)}" cy="${y(value(point, s.key))}" r="3.2" class="point-${suffix}"/>`
            : "",
        )
        .join("");
      return `<path d="${path}" class="series-${suffix}"/>${dots}`;
    })
    .join("");
  const labelCount = Math.min(points.length, Math.max(2, Math.floor(plotWidth / 100)));
  const labelIndices = new Set(
    Array.from({ length: labelCount }, (_, i) =>
      labelCount === 1 ? 0 : Math.round((i * (points.length - 1)) / (labelCount - 1)),
    ),
  );
  const labels = points
    .map((point, i) =>
      labelIndices.has(i)
        ? `<text x="${x(i)}" y="${height - 8}" text-anchor="middle" class="axis-label">${escapeHtml(point.date || "")}</text>`
        : "",
    )
    .join("");
  return `<svg class="chart" viewBox="0 0 ${width} ${height}" role="img" aria-label="${escapeHtml(spec.subtitle)}">${grid}${lines}${labels}</svg>`;
}

const resizeObserver = new ResizeObserver((entries) => {
  for (const entry of entries) {
    const region = entry.target;
    const spec = charts.get(region.dataset.chart);
    const width = Math.max(360, Math.round(entry.contentRect.width));
    if (!spec || !entry.contentRect.width || Number(region.dataset.width) === width) continue;
    region.dataset.width = String(width);
    region.innerHTML = activitySvg(spec, width);
  }
});

new MutationObserver((mutations) => {
  for (const mutation of mutations) {
    for (const node of mutation.removedNodes) {
      if (node.nodeType !== Node.ELEMENT_NODE) continue;
      for (const region of [node, ...node.querySelectorAll("[data-chart]")]) {
        if (region.matches("[data-chart]")) resizeObserver.unobserve(region);
      }
    }
  }
  for (const region of document.querySelectorAll("[data-chart]:not([data-observed])")) {
    region.dataset.observed = "";
    resizeObserver.observe(region);
  }
}).observe(document.body, { childList: true, subtree: true });

// `points` are daily rows with a `date`; `series` names the two plotted keys.
export function activityPanel(points, options = {}) {
  const {
    id = "activity",
    title = "Activity over time",
    subtitle = "Daily merged PRs and commits, UTC",
    series = SERIES,
    className = "",
    empty = "No activity in this range.",
  } = options;
  const spec = { points: points || [], series, subtitle };
  charts.set(id, spec);
  const legend = `<div class="legend">${series.map((s, index) => `<span><i class="dot${index ? " orange" : ""}"></i>${escapeHtml(s.label)}</span>`).join("")}</div>`;
  const active = spec.points.some((point) => series.some((s) => Number(point[s.key]) > 0));
  const body = active
    ? `<div class="chart-region" data-chart="${escapeHtml(id)}">${activitySvg(spec, 760)}</div>`
    : `<div class="empty">${escapeHtml(empty)}</div>`;
  return panel({
    title,
    subtitle,
    controls: legend,
    className: `activity-panel ${className}`,
    body,
  });
}

/* Share pies */

const pies = new Map();
const pieMetrics = new Map();

// A donut of `rows` by one metric, largest first, with a linked legend. `metrics` are
// { key, label, unit, subtitle? }; with more than one, a toggle switches between them and
// the choice is kept for that pie across re-renders. `link(row)` is a legend entry's HTML,
// `name(row)` its plain name, and `neutral(row)` marks a row drawn in the neutral color.
export function sharePie(spec) {
  pies.set(spec.id, spec);
  return renderPie(spec);
}

function renderPie(spec) {
  const {
    id,
    title,
    rows = [],
    metrics,
    link,
    name = (row) => row.name || "",
    neutral = () => false,
    note = "",
    large = false,
    empty = "No activity in this range.",
  } = spec;
  const metric = metrics.find((m) => m.key === pieMetrics.get(id)) || metrics[0];
  const slices = rows
    .map((row) => ({ row, value: Number(row[metric.key]) || 0 }))
    .filter((slice) => slice.value > 0)
    .sort((a, b) => b.value - a.value || name(a.row).localeCompare(name(b.row)))
    .map((slice, index) => ({
      ...slice,
      color: neutral(slice.row) ? NEUTRAL_CHART_COLOR : chartColor(index),
    }));
  const total = slices.reduce((sum, slice) => sum + slice.value, 0);
  const controls =
    metrics.length > 1
      ? toggle({
          label: `${title} metric`,
          options: metrics.map((m) => [m.key, m.label]),
          selected: metric.key,
        })
      : "";
  const radius = 48;
  const circumference = 2 * Math.PI * radius;
  let offset = 0;
  const share = (slice) => `${Math.round((slice.value / total) * 100)}%`;
  const circles = slices
    .map((slice, index) => {
      const length = (slice.value / total) * circumference;
      // A hovered slice moves outward along its middle; slices start at 12 o'clock.
      const middle = ((offset + length / 2) / circumference) * 2 * Math.PI;
      const dx = (Math.sin(middle) * 5).toFixed(2);
      const dy = (-Math.cos(middle) * 5).toFixed(2);
      const circle = `<g class="share-pie-slice" data-slice="${index}" data-value="${number.format(slice.value)}" data-share="${share(slice)}" style="--dx:${dx}px;--dy:${dy}px"><circle cx="64" cy="64" r="${radius}" fill="none" style="stroke:${slice.color}" stroke-width="24" stroke-dasharray="${length} ${circumference - length}" stroke-dashoffset="${-offset}" transform="rotate(-90 64 64)"><title>${escapeHtml(name(slice.row))}: ${number.format(slice.value)}</title></circle></g>`;
      offset += length;
      return circle;
    })
    .join("");
  const legend = slices
    .map(
      (slice, index) =>
        `<li data-slice="${index}"><span class="share-pie-key" style="--slice-color:${slice.color}"></span><span class="share-pie-name">${link(slice.row)}</span><span class="share-pie-value">${number.format(slice.value)} · ${share(slice)}</span></li>`,
    )
    .join("");
  const chart = total
    ? `<div class="share-pie${large ? " share-pie-large" : ""}"><svg viewBox="0 0 128 128" role="img" aria-label="${escapeHtml(`${title}: ${metric.subtitle || metric.label}`)}">${circles}<text x="64" y="61" text-anchor="middle" class="share-pie-total" data-total="${number.format(total)}">${number.format(total)}</text><text x="64" y="77" text-anchor="middle" class="share-pie-unit" data-unit="${escapeHtml(metric.unit)}">${escapeHtml(metric.unit)}</text></svg><ul class="share-pie-legend">${legend}</ul></div>`
    : `<div class="empty">${escapeHtml(empty)}</div>`;
  return panel({
    title,
    subtitle: metric.subtitle || spec.subtitle || "",
    controls,
    className: "share-pie-panel",
    attributes: `data-pie="${escapeHtml(id)}"`,
    body: chart + (note ? `<p class="subtitle">${escapeHtml(note)}</p>` : ""),
  });
}

document.addEventListener("click", (event) => {
  const button = event.target.closest("[data-pie] [data-toggle-value]");
  const section = button?.closest("[data-pie]");
  const spec = section && pies.get(section.dataset.pie);
  if (!spec) return;
  const value = button.dataset.toggleValue;
  pieMetrics.set(spec.id, value);
  section.outerHTML = renderPie(spec);
  document
    .querySelector(`[data-pie="${CSS.escape(spec.id)}"] [data-toggle-value="${CSS.escape(value)}"]`)
    ?.focus();
});

// Hovering or focusing a slice or its legend entry pops the slice out, fades the others,
// and shows its value and share in the middle of the donut.
function activateSlice(chart, index) {
  chart.classList.toggle("is-active", index != null);
  for (const element of chart.querySelectorAll("[data-slice]")) {
    element.classList.toggle("is-active", element.dataset.slice === index);
  }
  const slice = index != null && chart.querySelector(`g[data-slice="${index}"]`);
  const total = chart.querySelector(".share-pie-total");
  const unit = chart.querySelector(".share-pie-unit");
  total.textContent = slice ? slice.dataset.value : total.dataset.total;
  unit.textContent = slice ? slice.dataset.share : unit.dataset.unit;
  // Bring the legend entry into view inside a scrolling legend, without scrolling the page.
  const item = slice && chart.querySelector(`li[data-slice="${index}"]`);
  const legend = item?.parentElement;
  if (item && legend.scrollHeight > legend.clientHeight) {
    const top =
      item.getBoundingClientRect().top - legend.getBoundingClientRect().top + legend.scrollTop;
    const bottom = top + item.offsetHeight;
    if (top < legend.scrollTop) legend.scrollTop = top;
    else if (bottom > legend.scrollTop + legend.clientHeight) {
      legend.scrollTop = bottom - legend.clientHeight;
    }
  }
}

for (const type of ["mouseover", "focusin"]) {
  document.addEventListener(type, (event) => {
    const item = event.target.closest?.(".share-pie [data-slice]");
    if (item) activateSlice(item.closest(".share-pie"), item.dataset.slice);
  });
}
for (const type of ["mouseout", "focusout"]) {
  document.addEventListener(type, (event) => {
    const chart = event.target.closest?.(".share-pie");
    if (chart && !chart.contains(event.relatedTarget)) activateSlice(chart, null);
  });
}

/* Languages */

// A language breakdown as a stacked bar with a legend; `share` values are fractions.
export function languagePanel(title, languages, subtitle) {
  const rows = (languages || []).filter((row) => row.share > 0);
  if (!rows.length) {
    return panel({
      title,
      subtitle,
      className: "language-panel",
      body: '<div class="empty">No language data yet.</div>',
    });
  }
  const color = (row) => (/^#[0-9a-f]{3,8}$/i.test(row.color) ? row.color : NEUTRAL_CHART_COLOR);
  const percent = (share) => (share < 0.001 ? "<0.1%" : `${(share * 100).toFixed(1)}%`);
  const label = rows.map((row) => `${row.name} ${percent(row.share)}`).join(", ");
  const bar = rows
    .map(
      (row) =>
        `<span style="--language-color:${color(row)};flex-grow:${row.share}" title="${escapeHtml(row.name)} ${percent(row.share)}"></span>`,
    )
    .join("");
  const legend = rows
    .map(
      (row) =>
        `<li><span class="language-key" style="--language-color:${color(row)}"></span><span class="language-name">${escapeHtml(row.name)}</span><span class="language-share">${percent(row.share)}</span></li>`,
    )
    .join("");
  return panel({
    title,
    subtitle,
    className: "language-panel",
    body: `<div class="language-bar" role="img" aria-label="${escapeHtml(label)}">${bar}</div><ul class="language-legend">${legend}</ul>`,
  });
}
