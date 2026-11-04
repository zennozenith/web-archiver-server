import type { Alpine } from "alpinejs";

/** Inline rename for a tag row. The current name comes from `data-name` – never interpolate it into JS. */
export function registerTagEditor(alpine: Alpine): void {
  alpine.data("tagEditor", () => ({
    editing: false,
    name: "",
    start() {
      this.name = (this.$root as HTMLElement).dataset.name ?? "";
      this.editing = true;
      this.$nextTick(() => {
        const input = this.$refs.input as HTMLInputElement | undefined;
        input?.focus();
        input?.select();
      });
    },
    cancel() {
      this.editing = false;
    },
  }));
}
