import type { Alpine } from "alpinejs";

/** Client-side filter for the archived-page list. Cards carry a lower-cased `data-search`. */
export function registerPageFilter(alpine: Alpine): void {
  alpine.data("pageFilter", () => ({
    q: "",
    matches(search: string | undefined): boolean {
      return this.q === "" || (search ?? "").includes(this.q.trim().toLowerCase());
    },
    get noMatches(): boolean {
      if (this.q === "") return false;
      const cards = this.$root.querySelectorAll<HTMLElement>("[data-search]");
      return ![...cards].some((el) => this.matches(el.dataset.search));
    },
  }));
}
