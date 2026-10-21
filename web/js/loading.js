export const PulseLoading = {
  indicator(message = "Loading…", inline = false) {
    const label = document.createElement("span");
    label.textContent = message;
    return `<span class="loading-indicator${inline ? " loading-inline" : ""}" role="status"><span class="loading-spinner" aria-hidden="true"></span>${label.outerHTML}</span>`;
  },
};
