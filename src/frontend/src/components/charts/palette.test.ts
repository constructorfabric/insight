import { describe, expect, it } from "vitest";

import { OTHER_COLOR, colorKeys, seriesColor } from "./palette";

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
    const [other, api] = colorKeys([
      { key: "other", label: "Other" },
      { key: "s0", label: "api" },
    ]);

    expect(other?.color).toBe(OTHER_COLOR);
    expect(api?.color).toBe("var(--chart-2)");
  });
  it("keeps Other apart from every series colour", () => {
    const series = Array.from({ length: 12 }, (_, index) => seriesColor(index));

    expect(series).not.toContain(OTHER_COLOR);
    expect(OTHER_COLOR).not.toMatch(/--chart-|--viz-/);
  });
});
