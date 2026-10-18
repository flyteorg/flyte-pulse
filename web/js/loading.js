export const PulseLoading = {
  indicator(message = "Loading…", inline = false) {
    const label = document.createElement("span");
    label.textContent = message;
    return `<span class="loading-indicator${inline ? " loading-inline" : ""}" role="status"><span class="loading-spinner" aria-hidden="true"></span>${label.outerHTML}</span>`;
  },

  region(element, busy, message = "Loading…") {
    element.setAttribute("aria-busy", String(busy));
    element.classList.toggle("is-loading", busy);
    let placeholder = element.querySelector(":scope > .loading-state");
    if (busy) {
      if (!placeholder) {
        placeholder = document.createElement("div");
        placeholder.className = "loading-state";
        element.append(placeholder);
      }
      placeholder.innerHTML = this.indicator(message);
    } else {
      placeholder?.remove();
    }
  },

  refreshing(element, busy) {
    element.setAttribute("aria-busy", String(busy));
    element.classList.toggle("is-refreshing", busy);
  },
};
