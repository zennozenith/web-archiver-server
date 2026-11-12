import Alpine from "alpinejs";

import { registerLinkView } from "./alpine/link-view";
import { registerMagics } from "./alpine/magics";
import { registerPageFilter } from "./alpine/page-filter";
import { registerReader } from "./alpine/reader";
import { registerTagEditor } from "./alpine/tag-editor";
import { registerTheme } from "./alpine/theme";
import { registerToasts } from "./alpine/toasts";

registerMagics(Alpine);
registerTheme(Alpine);
registerToasts(Alpine);
registerTagEditor(Alpine);
registerPageFilter(Alpine);
registerLinkView(Alpine);
registerReader(Alpine);

declare global {
  interface Window {
    Alpine: typeof Alpine;
    htmx: any;
    setTheme: any;
  }
}
window.Alpine = Alpine;
Alpine.start();

// tiny client-side helpers (theme + local timestamps).
(() => {
  const THEME_KEY = "userdemo-theme";

  // Used by the theme toggle in base.html (Alpine).
  window.setTheme = (name: any) => {
    document.documentElement.setAttribute("data-theme", name);
    try {
      localStorage.setItem(THEME_KEY, name);
    } catch (_) {
      /* private mode */
    }
  };

  // The server renders timestamps in UTC; show them in the viewer's time zone.
  const format = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });

  function localize(root: any) {
    (root instanceof Element || root instanceof Document ? root : document)
      .querySelectorAll("time[data-local]")
      .forEach((el) => {
        const iso = el.getAttribute("datetime");
        if (!iso) return;
        const date = new Date(iso);
        if (Number.isNaN(date.getTime())) return;
        el.textContent = format.format(date);
        (el as HTMLElement).title = date.toISOString();
        el.removeAttribute("data-local");
      });
  }

  document.addEventListener("DOMContentLoaded", () => localize(document));
  // Content swapped in by htmx (tables, modal, OOB updates).
  if (window.htmx && typeof window.htmx.onLoad === "function") {
    window.htmx.onLoad(localize);
  }
  document.addEventListener("htmx:after:swap", () => localize(document));
})();
