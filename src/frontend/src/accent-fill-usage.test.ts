import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

const src = join(__dirname);
const ACCENT_FILL = /(?<![\w-])bg-accent(?![\w-])/;

function appSources(): string[] {
  return readdirSync(src, { recursive: true, encoding: "utf8" }).filter(
    (file) =>
      file.endsWith(".tsx") &&
      !file.startsWith(join("components", "ui")) &&
      !file.endsWith(".test.tsx") &&
      !file.endsWith(".stories.tsx")
  );
}

describe("accent fill", () => {
  it("is left to kit components, whose accent pairs with white type", () => {
    const offenders = appSources().filter((file) =>
      ACCENT_FILL.test(readFileSync(join(src, file), "utf8"))
    );

    expect(offenders, "should not paint bg-accent").toEqual([]);
  });
});
