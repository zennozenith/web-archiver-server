import Alpine from "@alpinejs/csp";

import { registerLinkView } from "./alpine/link-view";
import { registerMagics } from "./alpine/magics";
import { registerPageFilter } from "./alpine/page-filter";
import { registerReader } from "./alpine/reader";
import { registerTagEditor } from "./alpine/tag-editor";
import { registerTheme } from "./alpine/theme";
import { registerToasts } from "./alpine/toasts";
import { registerUi } from "./alpine/ui";

registerMagics(Alpine);
registerTheme(Alpine);
registerToasts(Alpine);
registerTagEditor(Alpine);
registerPageFilter(Alpine);
registerLinkView(Alpine);
registerReader(Alpine);
registerUi(Alpine);

declare global {
  interface Window {
    Alpine: typeof Alpine;
    htmx: any;
  }
}

// Apply the persisted theme before Alpine starts to avoid a flash of the default.
(() => {
  try {
    const theme = localStorage.getItem("userdemo-theme");
    if (theme) document.documentElement.setAttribute("data-theme", theme);
  } catch {
    /* private mode */
  }
})();

window.Alpine = Alpine;
Alpine.start();

// tiny client-side helpers (theme + local timestamps).
(() => {
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

  // Keep "Go back" links working without an inline `javascript:` URL.
  document.addEventListener("click", (event) => {
    const target = (event.target as Element | null)?.closest("[data-back]");
    if (!target) return;
    event.preventDefault();
    history.back();
  });
})();
