import type { Alpine } from "alpinejs";

export const THEMES = [
  { id: "minimaltide", label: "Minimal Tide" },
  { id: "chillecho", label: "Chill Echo" },
  { id: "lastpickle", label: "Last Pickle" },
  { id: "almostultra", label: "Almost Ultra" },
  { id: "dark", label: "Dark" },
  { id: "light", label: "Light" },
] as const;

const KEY = "theme";
const FALLBACK = "minimaltide";

function read(): string {
  try {
    return localStorage.getItem(KEY) || FALLBACK;
  } catch {
    return FALLBACK;
  }
}

export function registerTheme(alpine: Alpine): void {
  alpine.data("themeSwitcher", () => ({
    themes: THEMES,
    current: read(),
    set(id: string) {
      this.current = id;
      document.documentElement.dataset.theme = id;
      try {
        localStorage.setItem(KEY, id);
      } catch {
        /* storage unavailable – theme still applies for this page */
      }
    },
  }));
}
