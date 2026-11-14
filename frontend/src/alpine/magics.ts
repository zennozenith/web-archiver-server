import type { Alpine } from "alpinejs";

/**
 * `$ok($event)` – true when an htmx request finished with a status < 400.
 * Use it to guard follow-up work:  @htmx:after:request="$ok($event) && $el.reset()"
 */
export function registerMagics(alpine: Alpine): void {
  alpine.magic("ok", () => (event: Event): boolean => {
    const status = (event as CustomEvent).detail?.ctx?.response?.status;
    return typeof status === "number" && status < 400;
  });

  alpine.magic("reload", () => (event: Event): void => {
    if ((event as CustomEvent).detail?.ctx?.response?.status < 400) location.reload();
  });

  alpine.magic("navigate", () => (event: Event, url: string): void => {
    if ((event as CustomEvent).detail?.ctx?.response?.status < 400) location.href = url;
  });
}
