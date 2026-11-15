import type { Alpine } from "alpinejs";

const SIZE_KEY = "reader:size";
const MIN = 14;
const MAX = 28;

function loadSize(): number {
  try {
    const n = Number(localStorage.getItem(SIZE_KEY));
    return n >= MIN && n <= MAX ? n : 18;
  } catch {
    return 18;
  }
}

/**
 * State for the link detail / reader page.
 * Initial mode is read from `data-mode` ("view" | "edit") so no server value is evaluated as JS.
 */
export function registerReader(alpine: Alpine): void {
  alpine.data("reader", () => ({
    panel: true,
    mode: "view",
    format: "readable",
    size: loadSize(),
    selection: "",
    copied: false,

    init() {
      this.mode = (this.$root as HTMLElement).dataset.mode === "edit" ? "edit" : "view";
      // On narrow screens the panel starts closed so the article is visible.
      if (window.matchMedia("(max-width: 1023px)").matches && this.mode === "view") this.panel = false;
    },
    resize(delta: number) {
      this.size = Math.min(MAX, Math.max(MIN, this.size + delta));
      try {
        localStorage.setItem(SIZE_KEY, String(this.size));
      } catch {
        /* ignore */
      }
    },
    capture() {
      const text = window.getSelection()?.toString().trim() ?? "";
      if (text) this.selection = text;
    },
    clearSelection() {
      this.selection = "";
      window.getSelection()?.removeAllRanges();
    },
    highlightVals(color: string): string {
      return JSON.stringify({ color, text: this.selection });
    },
    async copy(text: string) {
      await navigator.clipboard.writeText(text);
      this.copied = true;
      setTimeout(() => (this.copied = false), 1500);
    },
  }));
}
