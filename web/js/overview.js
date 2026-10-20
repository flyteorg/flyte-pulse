import { getJson, snapshotMeta } from "./api.js";
import { NEUTRAL_CHART_COLOR, chartColor, formatStamp, languagePanel } from "./common.js";
import { PulseLoading } from "./loading.js";
import { currentRoute, replaceParams } from "./route.js";

const number = new Intl.NumberFormat();

// Render the overview into `root` and return a cleanup function for navigation away.
export function mountOverview(root) {
  root.replaceChildren(document.getElementById("overview-template").content.cloneNode(true));
  const $ = (id) => document.getElementById(id);
  let currentData = null;
  let contributorKind = "HUMAN";
  let repositoryPieMetric = "pull_requests_merged";
  let companyPieMetric = "pull_requests_merged";
  let contributorSearchQuery = currentRoute().params.get("q") || "";
  let aiRequestId = 0;
  let dashboardRequestId = 0;

  function setOverviewLoading(busy) {
    for (const id of ["contributors", "new-contributors", "merged", "commits", "opened"]) {
      const value = $(id);
      if (busy && !currentData) value.innerHTML = PulseLoading.indicator("Loading…", true);
      PulseLoading.refreshing(value.closest(".card"), busy && Boolean(currentData));
      value.closest(".card").setAttribute("aria-busy", String(busy));
    }
    for (const [id, message] of [
      ["chart-content", "Loading activity…"],
      ["contributors-content", "Loading contributors…"],
      ["repository-pie-content", "Loading repository share…"],
      ["repositories-content", "Loading repository activity…"],
    ]) {
      const region = $(id);
      if (busy && !currentData) PulseLoading.region(region, true, message);
      else if (
        !busy &&
        !(
          id === "contributors-content" &&
          contributorKind === "AI" &&
          currentData?.ai_status === "loading"
        )
      ) {
        PulseLoading.region(region, false);
      }
      PulseLoading.refreshing(region.closest(".panel"), busy && Boolean(currentData));
    }
  }
  function setOptions(select, items, allLabel, selected) {
    const old = selected ?? select.value;
    select.replaceChildren(new Option(allLabel, ""));
    for (const item of items) {
      const option = new Option(item.name, item.id);
      select.add(option);
    }
    select.value = [...select.options].some((o) => o.value === old) ? old : "";
  }
  function cell(row, value, className) {
    const td = document.createElement("td");
    if (className) td.className = className;
    td.textContent = value;
    row.append(td);
    return td;
  }
  function renderTable(target, empty, rows, columns) {
    const body = $(target);
    body.replaceChildren();
    for (const item of rows) {
      const tr = document.createElement("tr");
      for (const column of columns) {
        const value = item[column.key];
        const text = column.format ? column.format(value, item) : String(value ?? "—");
        const td = cell(tr, "", column.className || "");
        const href = column.href ? column.href(value, item) : "";
        if (href) {
          const anchor = document.createElement("a");
          anchor.className = "data-link";
          anchor.href = href;
          anchor.textContent = text;
          td.append(anchor);
        } else {
          td.textContent = text;
        }
      }
      body.append(tr);
    }
    $(empty).hidden = rows.length > 0;
  }
  function repositoryHref(repository) {
    const query = repository.target_branch
      ? `?target_branch=${encodeURIComponent(repository.target_branch)}`
      : "";
    return `#/repositories/${encodeURIComponent(repository.id)}${query}`;
  }
  function renderContributors(data) {
    const isAI = contributorKind === "AI";
    const isNew = contributorKind === "NEW";
    $("contributors-heading").textContent = isNew
      ? "New contributors"
      : `Top ${isAI ? "AI" : "human"} contributors`;
    $("contributors-search-label").textContent = isAI
      ? "Search by model name"
      : "Search by username or name";
    $("contributors-search").placeholder = isAI ? "Model name" : "Username or full name";
    $("contributors-subtitle").textContent = isAI
      ? "AI line changes are shared across each recognized AI identity"
      : isNew
        ? "First recorded contribution in the selected range and repository, newest first"
        : "Line changes are shared across the PR author and recognized co-authors";
    document.querySelectorAll("[data-contributor-kind]").forEach((button) => {
      button.setAttribute(
        "aria-pressed",
        String(button.dataset.contributorKind === contributorKind),
      );
    });
    if (isAI && data.ai_status !== "loaded") {
      $("contributors-search-status").textContent = "";
      $("contributors-table").replaceChildren();
      $("contributors-columns").replaceChildren();
      const failed = data.ai_status === "error";
      PulseLoading.region($("contributors-content"), !failed, "Loading AI contributor activity…");
      $("contributors-empty").textContent = data.ai_error || "";
      $("contributors-empty").hidden = !failed;
      return;
    }
    PulseLoading.region($("contributors-content"), false);
    const allRows =
      data[
        isAI ? "ai_top_contributors" : isNew ? "new_top_contributors" : "human_top_contributors"
      ] || [];
    const query = contributorSearchQuery.trim().toLocaleLowerCase();
    const matchingRows = query
      ? allRows.filter((item) =>
          `${item.login || ""} ${item.name || ""} ${item.provider || ""} ${item.model || ""} ${item.id || ""}`
            .toLocaleLowerCase()
            .includes(query),
        )
      : allRows;
    const rows = matchingRows.slice(0, 18);
    $("contributors-empty").textContent = isAI
      ? "No recognized AI attribution in this range."
      : isNew
        ? "No new contributors in this range."
        : "No human contributor activity in this range.";
    if (query) {
      $("contributors-empty").textContent =
        `No ${isAI ? "AI identities" : "human contributors"} match “${contributorSearchQuery.trim()}”.`;
    }
    const searchStatus = $("contributors-search-status");
    if (query) {
      searchStatus.textContent =
        matchingRows.length > rows.length
          ? `Showing ${number.format(rows.length)} of ${number.format(matchingRows.length)} matches`
          : `${number.format(matchingRows.length)} match${matchingRows.length === 1 ? "" : "es"}`;
    } else {
      searchStatus.textContent =
        allRows.length > rows.length
          ? `Showing the top ${number.format(rows.length)} of ${number.format(allRows.length)}`
          : `${number.format(allRows.length)} contributor${allRows.length === 1 ? "" : "s"}`;
    }
    const labels = isAI
      ? [
          "Provider",
          "Model",
          "Authored",
          "Co-authored",
          "Assisted",
          "PRs with AI",
          "Lines added",
          "Lines removed",
        ]
      : [
          "Contributor",
          ...(isNew ? ["First recorded"] : []),
          "Merged",
          "Commits",
          "Opened",
          "Lines added",
          "Lines removed",
        ];
    const header = $("contributors-columns");
    header.replaceChildren(
      ...labels.map((label, index) => {
        const th = document.createElement("th");
        th.textContent = label;
        if (index > (isAI ? 1 : isNew ? 1 : 0)) th.className = "num";
        return th;
      }),
    );
    const columns = isAI
      ? [
          {
            key: "provider",
            className: "name",
            href: (_, row) => {
              if (!row.provider_key) return "";
              const query = new URLSearchParams({
                provider: row.provider || "",
                model: "",
                name: row.provider || "",
              });
              return `#/ai-contributors/${encodeURIComponent(row.provider_key)}?${query.toString()}`;
            },
          },
          { key: "model", className: "name", format: (value) => value || "—" },
          { key: "commits_ai_authored", className: "num", format: number.format },
          { key: "commits_ai_coauthored", className: "num", format: number.format },
          { key: "commits_ai_assisted", className: "num", format: number.format },
          { key: "pull_requests_with_ai", className: "num", format: number.format },
          { key: "shared_additions", className: "num", format: number.format },
          { key: "shared_deletions", className: "num", format: number.format },
        ]
      : [
          {
            key: "name",
            className: "name",
            href: (_, row) => `#/users/${encodeURIComponent(row.id)}`,
          },
          ...(isNew ? [{ key: "first_contribution_date" }] : []),
          { key: "pull_requests_merged", className: "num", format: number.format },
          { key: "commits", className: "num", format: number.format },
          { key: "pull_requests_opened", className: "num", format: number.format },
          { key: "shared_additions", className: "num", format: number.format },
          { key: "shared_deletions", className: "num", format: number.format },
        ];
    renderTable("contributors-table", "contributors-empty", rows, columns);
  }
  async function loadAIContributors(data) {
    if (data.ai_status === "loading" || data.ai_status === "loaded") {
      renderContributors(data);
      return;
    }
    const requestId = ++aiRequestId;
    data.ai_status = "loading";
    renderContributors(data);
    try {
      const result = await getJson(`/api/ai-dashboard?${data.ai_params}`);
      if (requestId !== aiRequestId || currentData !== data) return;
      data.ai_top_contributors = result.ai_top_contributors || [];
      data.ai_status = "loaded";
      if (contributorKind === "AI") renderContributors(data);
    } catch (error) {
      if (requestId !== aiRequestId || currentData !== data) return;
      data.ai_status = "error";
      data.ai_error = `AI contributor activity could not be loaded: ${error.message}`;
      if (contributorKind === "AI") renderContributors(data);
    }
  }
  function drawChart(points) {
    const svg = $("chart"),
      empty = $("chart-empty"),
      ns = "http://www.w3.org/2000/svg";
    svg.replaceChildren();
    if (!(points || []).some((point) => point.pull_requests_merged || point.commits)) {
      empty.hidden = false;
      return;
    }
    empty.hidden = true;
    const width = Math.max(360, Math.round(svg.clientWidth || 760)),
      height = 230;
    const pad = { l: 36, r: 40, t: 15, b: 32 },
      plotW = width - pad.l - pad.r,
      plotH = height - pad.t - pad.b;
    svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
    const maximum = Math.max(1, ...points.map((p) => Math.max(p.pull_requests_merged, p.commits)));
    const step = maximum < 10 ? 2 : maximum < 40 ? 5 : 10;
    const scaleMax = Math.max(step, Math.ceil(maximum / step) * step);
    const add = (name, attrs) => {
      const el = document.createElementNS(ns, name);
      for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
      svg.append(el);
      return el;
    };
    for (let i = 0; i <= 4; i++) {
      const y = pad.t + (plotH * i) / 4,
        value = Math.round((scaleMax * (4 - i)) / 4);
      add("line", { x1: pad.l, y1: y, x2: width - pad.r, y2: y, class: "gridline" });
      const text = add("text", {
        x: pad.l - 8,
        y: y + 3,
        "text-anchor": "end",
        class: "axis-label",
      });
      text.textContent = number.format(value);
    }
    const x = (i) => pad.l + (points.length <= 1 ? plotW / 2 : (i * plotW) / (points.length - 1));
    const y = (value) => pad.t + plotH - (value / scaleMax) * plotH;
    for (const [key, cls, pointCls] of [
      ["pull_requests_merged", "series-a", "point-a"],
      ["commits", "series-b", "point-b"],
    ]) {
      const path = points.map((point, i) => `${i ? "L" : "M"}${x(i)},${y(point[key])}`).join(" ");
      if (path) add("path", { d: path, class: cls });
      points.forEach((point, i) => {
        if (point[key] > 0) add("circle", { cx: x(i), cy: y(point[key]), r: 3.2, class: pointCls });
      });
    }
    const labelCount = Math.min(points.length, Math.max(2, Math.floor(plotW / 100)));
    const labelIndices = new Set(
      Array.from({ length: labelCount }, (_, i) =>
        labelCount === 1 ? 0 : Math.round((i * (points.length - 1)) / (labelCount - 1)),
      ),
    );
    points.forEach((point, i) => {
      if (labelIndices.has(i)) {
        const text = add("text", {
          x: x(i),
          y: height - 8,
          "text-anchor": "middle",
          class: "axis-label",
        });
        text.textContent = point.date;
      }
    });
  }

  function drawRepositoryPie(repositories) {
    drawSharePie(
      "repository",
      repositories,
      repositoryPieMetric,
      repositoryPieMetric === "commits" ? "commits" : "merged PRs",
      repositoryHref,
    );
  }

  function drawCompanyPie(data) {
    if (!data.company_data) return;
    const rows = data.company_data.companies.map((company) => ({
      ...company,
      name: company.canonical_name,
    }));
    rows.sort(
      (a, b) =>
        (Number(b[companyPieMetric]) || 0) - (Number(a[companyPieMetric]) || 0) ||
        a.name.localeCompare(b.name),
    );
    drawSharePie(
      "company",
      rows,
      companyPieMetric,
      companyPieMetric === "commits" ? "commit credits" : "PR credits",
      (company) => `#/companies/${encodeURIComponent(company.id)}?${data.company_params}`,
    );
  }

  async function loadCompanyShare(data, params, requestId) {
    data.company_params = params.toString();
    try {
      const result = await getJson(`/api/companies?${params}`);
      if (requestId !== dashboardRequestId || currentData !== data) return;
      data.company_data = result;
      $("company-pie-empty").textContent = "No company activity in this range.";
      drawCompanyPie(data);
    } catch (error) {
      if (requestId !== dashboardRequestId || currentData !== data) return;
      $("company-pie").replaceChildren();
      $("company-pie-legend").replaceChildren();
      $("company-pie-empty").textContent =
        `Company contributions could not be loaded: ${error.message}`;
      $("company-pie-empty").hidden = false;
    } finally {
      if (requestId === dashboardRequestId && currentData === data)
        PulseLoading.region($("company-pie-content"), false);
    }
  }

  function drawSharePie(prefix, rows, metric, metricLabel, href) {
    const svg = $(`${prefix}-pie`),
      legend = $(`${prefix}-pie-legend`),
      empty = $(`${prefix}-pie-empty`);
    svg.replaceChildren();
    legend.replaceChildren();
    const entries = (rows || [])
      .map((repository) => ({ ...repository, value: Number(repository[metric]) || 0 }))
      .filter((repository) => repository.value > 0);
    const total = entries.reduce((sum, repository) => sum + repository.value, 0);
    empty.hidden = total > 0;
    if (!total) return;
    const ns = "http://www.w3.org/2000/svg",
      cx = 100,
      cy = 100,
      radius = 86,
      innerRadius = 52;
    const add = (name, attrs) => {
      const el = document.createElementNS(ns, name);
      for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
      svg.append(el);
      return el;
    };
    let angle = -Math.PI / 2;
    entries.forEach((repository, index) => {
      const color =
        prefix === "company" && repository.id === "independent"
          ? NEUTRAL_CHART_COLOR
          : chartColor(index);
      const sweep = (repository.value / total) * Math.PI * 2;
      let slice;
      if (sweep > Math.PI * 2 - 0.0001) {
        slice = add("circle", { cx, cy, r: radius, style: `fill:${color}` });
      } else {
        const endAngle = angle + sweep;
        const startX = cx + radius * Math.cos(angle),
          startY = cy + radius * Math.sin(angle);
        const endX = cx + radius * Math.cos(endAngle),
          endY = cy + radius * Math.sin(endAngle);
        const largeArc = sweep > Math.PI ? 1 : 0;
        slice = add("path", {
          d: `M ${cx} ${cy} L ${startX} ${startY} A ${radius} ${radius} 0 ${largeArc} 1 ${endX} ${endY} Z`,
          style: `fill:${color}`,
        });
        angle = endAngle;
      }
      slice.setAttribute("aria-label", `${repository.name}: ${number.format(repository.value)}`);
      slice.classList.add("pie-slice");

      const item = document.createElement("li");
      const key = document.createElement("span");
      key.className = "repository-pie-key";
      key.style.setProperty("--slice-color", color);
      const name = document.createElement("span");
      name.className = "repository-pie-name";
      const link = document.createElement("a");
      link.className = "data-link";
      link.href = href(repository);
      link.textContent = repository.name;
      link.title = repository.name;
      name.append(link);
      const value = document.createElement("span");
      value.className = "repository-pie-value";
      value.textContent = `${number.format(repository.value)} · ${Math.round((repository.value / total) * 100)}%`;
      item.append(key, name, value);
      legend.append(item);
    });
    add("circle", { cx, cy, r: innerRadius, class: "pie-hole" });
    const centerValue = add("text", {
      x: cx,
      y: cy - 2,
      "text-anchor": "middle",
      class: "repository-pie-total",
    });
    centerValue.textContent = number.format(total);
    const centerLabel = add("text", {
      x: cx,
      y: cy + 16,
      "text-anchor": "middle",
      class: "repository-pie-total-label",
    });
    centerLabel.textContent = metricLabel;
  }

  async function loadData() {
    const requestId = ++dashboardRequestId;
    const error = document.getElementById("error");
    error.style.display = "none";
    $("status").innerHTML = PulseLoading.indicator(
      currentData ? "Updating activity…" : "Loading activity…",
      true,
    );
    setOverviewLoading(true);
    PulseLoading.region($("company-pie-content"), true, "Loading company contributions…");
    const params = new URLSearchParams({ days: $("days").value });
    if ($("repository").value) params.set("repository_id", $("repository").value);
    try {
      const data = await getJson(`/api/dashboard?${params}`);
      if (requestId !== dashboardRequestId) return;
      data.ai_params = params.toString();
      currentData = data;
      void loadCompanyShare(data, params, requestId);
      setOptions(
        $("repository"),
        data.repositories.map((r) => ({ id: r.id, name: r.name })),
        "All repositories",
        $("repository").value,
      );
      $("chart-subtitle").textContent =
        "Daily merged PRs and commits across selected repositories, UTC";
      const s = data.summary;
      $("contributors").textContent = number.format(s.active_human_contributors);
      $("new-contributors").textContent = number.format(s.new_human_contributors);
      $("new-contributors-link").href = `#/users?${new URLSearchParams({
        days: $("days").value,
        ...($("repository").value ? { repository_id: $("repository").value } : {}),
        section: "new-contributors",
      })}`;
      $("merged").textContent = number.format(s.pull_requests_merged);
      $("commits").textContent = number.format(s.commits);
      $("opened").textContent = number.format(s.pull_requests_opened);
      $("chart-empty").textContent = "No activity in this range.";
      $("repository-pie-empty").textContent = "No repository activity in this range.";
      $("repositories-empty").textContent = "No repository activity in this range.";
      for (const id of ["chart-content", "repository-pie-content", "repositories-content"])
        PulseLoading.region($(id), false);
      drawChart(data.daily);
      drawRepositoryPie(data.repository_activity);
      $("languages-content").innerHTML = languagePanel(
        "Languages",
        data.languages,
        "Current code in the selected repositories, from GitHub’s language detection",
      );
      if (contributorKind === "AI") void loadAIContributors(data);
      else renderContributors(data);
      renderTable("repositories-table", "repositories-empty", data.repository_activity, [
        {
          key: "name",
          className: "name",
          href: (_, row) => repositoryHref(row),
        },
        { key: "pull_requests_merged", className: "num", format: number.format },
        { key: "commits", className: "num", format: number.format },
        { key: "pull_requests_opened", className: "num", format: number.format },
        {
          key: "additions",
          className: "num",
          format: (v, r) => `${number.format(v)} / ${number.format(r.deletions)}`,
        },
      ]);
      $("page-stamp").textContent = `Metrics updated ${formatStamp(data.metrics_generated_at)}`;
      $("artifact-info").textContent =
        `Dataset ${data.serving_version || data.metrics_version || snapshotMeta().version || "unknown"} · ${number.format(s.active_repositories)} active repositories`;
      $("status").textContent = `${data.repositories.length} repositories`;
    } catch (e) {
      if (requestId !== dashboardRequestId) return;
      error.textContent = e.message;
      error.style.display = "block";
      $("status").textContent = currentData
        ? "Update failed; showing previous results"
        : "Activity unavailable";
      PulseLoading.region($("company-pie-content"), false);
      if (currentData?.company_data) drawCompanyPie(currentData);
      else {
        $("company-pie-empty").textContent = "Company contributions could not be loaded.";
        $("company-pie-empty").hidden = false;
      }
      if (!currentData) {
        for (const id of ["contributors", "new-contributors", "merged", "commits", "opened"])
          $(id).textContent = "—";
        for (const id of [
          "chart-empty",
          "contributors-empty",
          "repository-pie-empty",
          "repositories-empty",
        ]) {
          $(id).textContent = "Activity could not be loaded.";
          $(id).hidden = false;
        }
        $("page-stamp").textContent = "Activity unavailable";
        $("artifact-info").textContent = "Activity could not be loaded";
      }
    } finally {
      if (requestId === dashboardRequestId) setOverviewLoading(false);
    }
  }

  $("days").addEventListener("change", () => loadData());
  $("repository").addEventListener("change", () => loadData());
  document.querySelectorAll("[data-contributor-kind]").forEach((button) =>
    button.addEventListener("click", () => {
      contributorKind = button.dataset.contributorKind;
      $("contributors-heading").textContent =
        contributorKind === "NEW"
          ? "New contributors"
          : `Top ${contributorKind === "AI" ? "AI" : "human"} contributors`;
      document
        .querySelectorAll("[data-contributor-kind]")
        .forEach((option) => option.setAttribute("aria-pressed", String(option === button)));
      if (currentData) {
        if (contributorKind === "AI") void loadAIContributors(currentData);
        else renderContributors(currentData);
      }
    }),
  );
  document.querySelectorAll("[data-pie-metric]").forEach((button) =>
    button.addEventListener("click", () => {
      repositoryPieMetric = button.dataset.pieMetric;
      document
        .querySelectorAll("[data-pie-metric]")
        .forEach((option) => option.setAttribute("aria-pressed", String(option === button)));
      $("repository-pie-subtitle").textContent =
        repositoryPieMetric === "commits"
          ? "Commits by repository"
          : "Merged pull requests by repository";
      if (currentData) drawRepositoryPie(currentData.repository_activity);
    }),
  );
  document.querySelectorAll("[data-company-pie-metric]").forEach((button) =>
    button.addEventListener("click", () => {
      companyPieMetric = button.dataset.companyPieMetric;
      document
        .querySelectorAll("[data-company-pie-metric]")
        .forEach((option) => option.setAttribute("aria-pressed", String(option === button)));
      $("company-pie-subtitle").textContent =
        companyPieMetric === "commits"
          ? "Shared commit credits by company"
          : "Shared merged PR credits by company";
      if (currentData) drawCompanyPie(currentData);
    }),
  );
  $("contributors-search").value = contributorSearchQuery;
  $("contributors-search").addEventListener("input", (event) => {
    contributorSearchQuery = event.currentTarget.value;
    const params = currentRoute().params;
    if (contributorSearchQuery.trim()) params.set("q", contributorSearchQuery.trim());
    else params.delete("q");
    replaceParams(params);
    if (currentData) renderContributors(currentData);
  });
  const onResize = () => {
    if (currentData) drawChart(currentData.daily);
  };
  window.addEventListener("resize", onResize);
  loadData();

  return () => {
    // Invalidate in-flight requests so they cannot write into the next page.
    dashboardRequestId++;
    aiRequestId++;
    window.removeEventListener("resize", onResize);
  };
}
