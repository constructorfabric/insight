import { describe, expect, it } from "vitest";

import { uiKitLayer } from "./vite.ui-kit-layer";

const KIT_CSS = "/app/node_modules/@gears-frontx/ui-kit/dist/card.css";

function transform(code: string, id: string): string | null {
  const plugin = uiKitLayer();
  const hook = plugin.transform as (
    this: unknown,
    code: string,
    id: string
  ) => { code: string } | null;
  return hook.call(null, code, id)?.code ?? null;
}

describe("uiKitLayer", () => {
  it("wraps kit CSS in the components layer", () => {
    const out = transform("._card{padding:1rem}", KIT_CSS);
    expect(out).toContain("@layer components {\n._card{padding:1rem}\n}");
  });

  it("declares the layer order before the layer it uses", () => {
    const out = transform("._card{padding:1rem}", KIT_CSS) ?? "";
    const order = out.indexOf(
      "@layer properties, theme, base, components, utilities;"
    );
    expect(order).toBeGreaterThanOrEqual(0);
    expect(order).toBeLessThan(out.indexOf("@layer components {"));
  });

  it("leaves everything else alone", () => {
    expect(transform("._x{padding:1rem}", "/app/src/index.css")).toBeNull();
    expect(
      transform(
        "export default {}",
        "/app/node_modules/@gears-frontx/ui-kit/dist/index.js"
      )
    ).toBeNull();
  });
});
