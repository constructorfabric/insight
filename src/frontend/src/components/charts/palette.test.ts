import { describe, expect, it } from "vitest";

import { OTHER_COLOR, keyColor, seriesColor } from "./palette";

describe("palette", () => {
  it.each([
    [0, "var(--chart-1)"],
    [5, "var(--chart-6)"],
    [11, "var(--chart-12)"],
    [12, "var(--chart-1)"],
  ])(
    "paints series %i from the chart tokens, cycling after twelve",
    (index, color) => {
      expect(seriesColor(index)).toBe(color);
    }
  );

  it("paints Other in its own muted colour whatever its position", () => {
    expect(keyColor({ key: "other", label: "Other" }, 0)).toBe(OTHER_COLOR);
    expect(keyColor({ key: "s0", label: "api" }, 0)).toBe("var(--chart-1)");
  });
});
