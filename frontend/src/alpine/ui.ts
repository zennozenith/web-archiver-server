import type { Alpine } from "alpinejs";

function requestSucceeded(event: Event): boolean {
  const status = (event as CustomEvent).detail?.ctx?.response?.status;
  return typeof status === "number" && status < 400;
}

/** Small components kept free of global lookups so the Alpine CSP build can run them. */
export function registerUi(alpine: Alpine): void {
  alpine.data("themeToggle", () => ({
    night: document.documentElement.dataset.theme === "ledger-night",
    get label() {
      return this.night ? "Switch to light theme" : "Switch to dark theme";
    },
    toggle() {
      this.night = !this.night;
      const theme = this.night ? "ledger-night" : "ledger";
      document.documentElement.dataset.theme = theme;
      try {
        localStorage.setItem("userdemo-theme", theme);
      } catch {
        /* storage unavailable – theme still applies for this page */
      }
    },
  }));

  alpine.data("newKey", () => ({
    copied: false,
    timer: 0,
    reset() {
      (document.getElementById("new-key-form") as HTMLFormElement | null)?.reset();
    },
    async copy() {
      const key = this.$refs.key as HTMLElement;
      await navigator.clipboard.writeText(key.textContent?.trim() ?? "");
      this.copied = true;
      clearTimeout(this.timer);
      this.timer = setTimeout(() => (this.copied = false), 1500);
    },
  }));

  alpine.data("toastMessage", () => ({
    init() {
      setTimeout(() => this.$el.remove(), 4500);
    },
    dismiss() {
      (this.$el as HTMLElement).remove();
    },
  }));

  alpine.data("dialogModal", () => ({
    show() {
      const dialog = (this.$el as HTMLElement).closest("dialog");
      if (dialog && !dialog.open) dialog.showModal();
    },
    close() {
      (this.$el as HTMLElement).closest("dialog")?.close();
    },
  }));

  alpine.data("imageFallback", () => ({
    init() {
      const image = this.$el as HTMLImageElement;
      if (image.complete && !image.naturalWidth) image.remove();
    },
    remove() {
      (this.$el as HTMLElement).remove();
    },
  }));

  alpine.data("pendingItem", () => ({
    editing: false,
    toggle() {
      this.editing = !this.editing;
    },
    reload(event: Event) {
      if (requestSucceeded(event)) location.reload();
    },
  }));
}
