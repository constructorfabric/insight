import { describe, expect, it } from "vitest";

import { fairWidths, typicalWidth, type ColumnNeed } from "./evidence-widths";

const need = (natural: number, minimum: number): ColumnNeed => ({
  natural,
  minimum,
});

const rounded = (widths: number[]) => widths.map((width) => Math.round(width));

describe("fairWidths", () => {
  it.each([
    [
      "a row with room for every column spreads the spare by natural width",
      [need(100, 50), need(300, 50)],
      800,
      [200, 600],
    ],
    [
      "a crowded row cuts only the widest, to one shared width",
      [need(100, 80), need(500, 80), need(400, 80)],
      700,
      [100, 300, 300],
    ],
    [
      "a short column keeps its natural width while long ones give way",
      [need(60, 60), need(900, 120)],
      400,
      [60, 340],
    ],
    [
      "a row narrower than every minimum keeps the minimums and scrolls",
      [need(300, 150), need(300, 150), need(300, 150)],
      300,
      [150, 150, 150],
    ],
    [
      "a column cut short never goes below its minimum",
      [need(1000, 200), need(1000, 50)],
      300,
      [200, 100],
    ],
  ] as const)("%s", (_case, needs, available, expected) => {
    expect(rounded(fairWidths(needs, available)), `case: ${_case}`).toEqual(
      expected
    );
  });

  it("fills the row exactly whenever the minimums fit", () => {
    const widths = fairWidths(
      [need(120, 90), need(640, 90), need(260, 90), need(410, 90)],
      900
    );

    expect(Math.round(widths.reduce((sum, width) => sum + width, 0))).toBe(900);
  });
});

describe("typicalWidth", () => {
  it.each([
    ["no values", [], 0],
    ["one value", [40], 40],
    ["an outlier among ten", [10, 10, 10, 10, 10, 10, 10, 10, 10, 900], 10],
    ["evenly spread values", [10, 20, 30, 40, 50, 60, 70, 80, 90, 100], 90],
  ] as const)(
    "takes the width nine in ten fit for %s",
    (_case, widths, expected) => {
      expect(typicalWidth(widths), `case: ${_case}`).toBe(expected);
    }
  );
});
