// Hash routes keep the site working under any GitHub Pages path: #/users/<id>?days=30.

function decodeSegment(segment) {
  try {
    return decodeURIComponent(segment);
  } catch {
    return segment;
  }
}

export function currentRoute() {
  const raw = location.hash.replace(/^#/, "");
  const queryStart = raw.indexOf("?");
  const pathPart = queryStart >= 0 ? raw.slice(0, queryStart) : raw;
  const query = queryStart >= 0 ? raw.slice(queryStart + 1) : "";
  const path = pathPart.startsWith("/") ? pathPart : `/${pathPart}`;
  return {
    path,
    parts: path.split("/").filter(Boolean).map(decodeSegment),
    params: new URLSearchParams(query),
  };
}

export function routeHref(path, params) {
  const query = params ? params.toString() : "";
  return `#${path}${query ? `?${query}` : ""}`;
}

// Update the query of the current route without a re-render. replaceState does not fire
// hashchange, so only explicit navigation renders a page.
export function replaceParams(params) {
  history.replaceState(history.state, "", routeHref(currentRoute().path, params));
}
