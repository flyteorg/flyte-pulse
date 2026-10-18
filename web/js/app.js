import init, { Pulse } from "../pkg/pulse_web.js";
import { setPulse, snapshotMeta } from "./api.js";
import { loadBrowsePage } from "./browse.js";
import { content, errorBox, formatStamp, page } from "./common.js";
import { PulseLoading } from "./loading.js";
import { mountOverview } from "./overview.js";
import { currentRoute } from "./route.js";

const REFRESH_INTERVAL_MS = 30 * 60 * 1000;
let ready = false;
let loadedVersion = "";
let lastRefreshCheck = Date.now();
let cleanupPage = null;

async function fetchMeta() {
  const response = await fetch("data/meta.json", { cache: "no-store" });
  if (!response.ok) {
    throw new Error(`Activity metadata could not be loaded (HTTP ${response.status})`);
  }
  return response.json();
}

async function loadSnapshot(meta) {
  const file = meta.file || "pulse.bin.gz";
  const response = await fetch(`data/${file}?v=${encodeURIComponent(meta.version || "")}`);
  if (!response.ok) {
    throw new Error(`Activity snapshot could not be loaded (HTTP ${response.status})`);
  }
  const bytes = new Uint8Array(await response.arrayBuffer());
  try {
    return Pulse.load(bytes);
  } catch (error) {
    throw new Error(error?.message || String(error), { cause: error });
  }
}

function datasetLabel() {
  return `Dataset ${loadedVersion || "unknown"}`;
}

function activate(pulse, meta) {
  setPulse(pulse);
  const details = snapshotMeta();
  // Compare refreshes against meta.json, which names the published snapshot file.
  loadedVersion = meta.version || details.version || "";
  document.getElementById("page-stamp").textContent =
    `Metrics updated ${formatStamp(details.generated_at || meta.generated_at)}`;
  document.getElementById("artifact-info").textContent = datasetLabel();
}

function updateNav(section) {
  const navSection = section === "ai-contributors" ? "ai-activity" : section;
  for (const item of document.querySelectorAll(".site-nav a")) {
    const target = item.getAttribute("href").replace(/^#\/?/, "");
    if (target === navSection) item.setAttribute("aria-current", "page");
    else item.removeAttribute("aria-current");
  }
}

function showFailure(message) {
  content.replaceChildren();
  errorBox.textContent = message;
  errorBox.style.display = "block";
  document.getElementById("page-stamp").textContent = "Activity unavailable";
  document.getElementById("artifact-info").textContent = "Activity could not be loaded";
}

async function render({ preserveScroll = false } = {}) {
  if (!ready) return;
  cleanupPage?.();
  cleanupPage = null;
  const requestId = ++page.requestId;
  const route = currentRoute();
  const section = route.parts[0] || "";
  const scrollY = window.scrollY;
  errorBox.style.display = "none";
  errorBox.textContent = "";
  document.getElementById("page-loading-status").hidden = true;
  updateNav(section);
  document.title = section ? `Flyte Pulse · ${section}` : "Flyte Pulse";
  if (!section) {
    content.setAttribute("aria-busy", "false");
    cleanupPage = mountOverview(content);
    return;
  }
  content.setAttribute("aria-busy", "true");
  content.innerHTML = `<section class="panel page-section"><div class="loading-state">${PulseLoading.indicator("Loading activity…")}</div></section>`;
  try {
    await loadBrowsePage(route, requestId);
    if (requestId !== page.requestId) return;
    document.getElementById("artifact-info").textContent = datasetLabel();
    if (preserveScroll) window.scrollTo(0, scrollY);
  } catch (error) {
    if (requestId !== page.requestId) return;
    showFailure(error.message);
  } finally {
    if (requestId === page.requestId) content.setAttribute("aria-busy", "false");
  }
}

async function refresh() {
  lastRefreshCheck = Date.now();
  if (!ready) return;
  try {
    const meta = await fetchMeta();
    if (!meta.version || meta.version === loadedVersion) return;
    const pulse = await loadSnapshot(meta);
    activate(pulse, meta);
    await render({ preserveScroll: true });
  } catch (error) {
    // Keep showing the loaded snapshot; the next check retries.
    console.warn("Flyte Pulse refresh failed", error);
  }
}

async function start() {
  window.addEventListener("hashchange", () => {
    window.scrollTo(0, 0);
    void render();
  });
  window.setInterval(() => void refresh(), REFRESH_INTERVAL_MS);
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden && Date.now() - lastRefreshCheck >= REFRESH_INTERVAL_MS) void refresh();
  });
  try {
    const [meta] = await Promise.all([fetchMeta(), init()]);
    activate(await loadSnapshot(meta), meta);
    ready = true;
  } catch (error) {
    content.setAttribute("aria-busy", "false");
    showFailure(`Activity could not be loaded: ${error?.message || error}`);
    return;
  }
  await render();
}

void start();
