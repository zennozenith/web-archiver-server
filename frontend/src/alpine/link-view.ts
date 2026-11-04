import type { Alpine } from "alpinejs";

type View = "grid" | "list";
const KEY = "links:view";

function load(): View {
  try {
    return localStorage.getItem(KEY) === "list" ? "list" : "grid";
  } catch {
    return "grid";
  }
}

/** Grid/list toggle for the links page, remembered per browser. */
export function registerLinkView(alpine: Alpine): void {
  alpine.data("linkView", () => ({
    view: load() as View,
    set(view: View) {
      this.view = view;
      try {
        localStorage.setItem(KEY, view);
      } catch {
        /* ignore */
      }
    },
  }));
}
