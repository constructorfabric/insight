import type { Plugin } from "vite";

const KIT = "@gears-frontx/ui-kit";

// TODO(#2705): drop this once the kit ships its CSS in a layer.
// Unlayered kit CSS beats Tailwind's layered utilities at any specificity, so a
// call site's className is silently dropped without this. Theme tokens are
// @imported from index.css and must stay unlayered.
export function uiKitLayer(): Plugin {
  return {
    name: "ui-kit-layer",
    enforce: "pre",
    transform(code, id) {
      const file = id.split("?")[0];
      if (!file.includes(KIT) || !file.endsWith(".css")) return null;
      return { code: `@layer components {\n${code}\n}`, map: null };
    },
  };
}
