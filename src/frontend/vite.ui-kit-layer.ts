import type { Plugin } from "vite";

const KIT = "@gears-frontx/ui-kit";

// The kit ships unlayered CSS, which beats Tailwind's layered utilities at any
// specificity: without this, a call site's className is silently dropped
// wherever the kit declares the same property. Theme tokens arrive through an
// @import in index.css and must stay unlayered.
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
