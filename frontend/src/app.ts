import Alpine from "alpinejs";

interface TagEditorState {
  editing: boolean;
  name: string;
  start(
    this: TagEditorState & {
      $nextTick(callback: () => void): void;
      $refs: Record<string, HTMLInputElement>;
    },
    name: string,
  ): void;
  cancel(): void;
}

function tagEditor(): TagEditorState {
  return {
    editing: false,
    name: "",
    start(name: string) {
      this.name = name;
      this.editing = true;
      this.$nextTick(() => {
        const input = this.$refs.input;
        input?.focus();
        input?.select();
      });
    },
    cancel() {
      this.editing = false;
    },
  };
}

Alpine.data("tagEditor", tagEditor);

declare global {
  interface Window {
    Alpine: typeof Alpine;
  }
}
window.Alpine = Alpine;
Alpine.start();
