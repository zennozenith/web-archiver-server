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
  }
}
window.Alpine = Alpine;
Alpine.start();
