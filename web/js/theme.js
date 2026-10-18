// Light/dark theme like flyte.org: dark by default, remembered in this browser. The inline
// script in index.html applies a saved theme before the first paint, using the same key.
const STORAGE_KEY = "pulse-theme";
const THEME_COLORS = { dark: "#050310", light: "#ffffff" };

function currentTheme() {
  return document.documentElement.dataset.theme === "light" ? "light" : "dark";
}

function applyTheme(theme) {
  document.documentElement.dataset.theme = theme;
  document.querySelector('meta[name="theme-color"]')?.setAttribute("content", THEME_COLORS[theme]);
  const next = theme === "dark" ? "light" : "dark";
  const button = document.getElementById("theme-toggle");
  button?.setAttribute("aria-label", `Switch to ${next} theme`);
  button?.setAttribute("title", `Switch to ${next} theme`);
}

export function initTheme() {
  applyTheme(currentTheme());
  document.getElementById("theme-toggle")?.addEventListener("click", () => {
    const theme = currentTheme() === "dark" ? "light" : "dark";
    applyTheme(theme);
    try {
      localStorage.setItem(STORAGE_KEY, theme);
    } catch {
      // Storage can be unavailable (private windows, blocked site data).
    }
  });
}
