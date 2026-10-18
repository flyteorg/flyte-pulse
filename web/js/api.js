// The dashboard API now runs in WebAssembly. Pages call HTTP-style API URLs, and
// this shim answers them from the loaded snapshot.

let pulse = null;
let meta = {};

export function setPulse(instance) {
  const previous = pulse;
  pulse = instance;
  // Release the replaced snapshot's WebAssembly memory. Queries run synchronously, so no
  // request can still be using it.
  if (previous && previous !== instance) previous.free?.();
  try {
    meta = JSON.parse(instance.meta());
  } catch {
    meta = {};
  }
}

export function snapshotMeta() {
  return meta;
}

export function todayUtc() {
  return new Date().toISOString().slice(0, 10);
}

export async function getJson(url) {
  // Yield first so loading states can paint before a longer query runs.
  await new Promise((resolve) => setTimeout(resolve, 0));
  if (!pulse) throw new Error("Activity data is still loading");
  let body;
  try {
    body = pulse.request(url, todayUtc());
  } catch (error) {
    const failure = new Error(error?.message || String(error) || "Unable to load dashboard data");
    failure.status = Number(error?.status) || 500;
    throw failure;
  }
  return JSON.parse(body);
}
