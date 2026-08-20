import type { Plugin } from "vite";

const KIT = "@gears-frontx/ui-kit";

// The kit ships unlayered CSS. Tailwind emits its utilities into @layer
// utilities, and an unlayered declaration beats every layered one whatever the
// specificity — so without this, any utility a call site passes is silently
// dropped wherever the kit declares the same property. @layer components is
// declared ahead of utilities, so moving the kit there restores the override.
// Theme tokens are @imported from index.css and must stay unlayered.
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
