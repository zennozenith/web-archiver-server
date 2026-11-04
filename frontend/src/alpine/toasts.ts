import type { Alpine } from "alpinejs";

/** Alerts swapped into #error-container disappear after a while, or on click. */
export function registerToasts(alpine: Alpine): void {
  alpine.data("toastRegion", () => ({
    init() {
      new MutationObserver((records) => {
        for (const record of records) {
          record.addedNodes.forEach((node) => {
            if (node instanceof HTMLElement) setTimeout(() => node.remove(), 8000);
          });
        }
      }).observe(this.$el, { childList: true });
    },
    dismiss(event: Event) {
      (event.target as HTMLElement).closest("[role=alert]")?.remove();
    },
  }));
}
