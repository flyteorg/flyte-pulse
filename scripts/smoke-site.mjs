// Start the built site's WebAssembly module in Node, load its snapshot, and answer one
// query, so a build that cannot start in the browser fails before it is deployed.
// Usage: node scripts/smoke-site.mjs [OUT_DIR]
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const out = resolve(process.argv[2] || "dist");
const { initSync, Pulse } = await import(pathToFileURL(join(out, "pkg/pulse_web.js")).href);
initSync({ module: readFileSync(join(out, "pkg/pulse_web_bg.wasm")) });
const meta = JSON.parse(readFileSync(join(out, "data/meta.json"), "utf8"));
const pulse = Pulse.load(readFileSync(join(out, "data", meta.file || "pulse.bin.gz")));
const today = new Date().toISOString().slice(0, 10);
const dashboard = JSON.parse(pulse.request("/api/dashboard?days=30", today));
if (!dashboard.summary) throw new Error("the dashboard query returned no summary");
console.log(`Smoke test passed: ${dashboard.repositories.length} repositories`);
