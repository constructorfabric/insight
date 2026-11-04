import { describe, expect, it } from "vitest";

import { fitWidths, type ColumnNeed } from "./evidence-widths";

const text = (natural: number): ColumnNeed => ({ natural, stretches: true });
const fixed = (natural: number): ColumnNeed => ({ natural, stretches: false });

const rounded = (widths: number[]) => widths.map((width) => Math.round(width));

describe("fitWidths", () => {
  it.each([
    [
      "spare room goes to the text columns by their width",
      [text(100), fixed(80), text(300)],
      880,
      [200, 80, 600],
    ],
    [
      "spare room goes to every column when none is text",
      [fixed(100), fixed(100)],
      400,
      [200, 200],
    ],
    [
      "a row wider than the room keeps every width and scrolls",
      [text(500), fixed(80), text(400)],
      600,
      [500, 80, 400],
    ],
    [
      "a row that fits exactly is left alone",
      [text(300), fixed(100)],
      400,
      [300, 100],
    ],
    [
      "text columns with no width share the room equally",
      [text(0), text(0), fixed(50)],
      250,
      [100, 100, 50],
    ],
  ] as const)("%s", (_case, needs, available, expected) => {
    expect(rounded(fitWidths(needs, available)), `case: ${_case}`).toEqual(
      expected
    );
  });

  it("never narrows a column below its natural width", () => {
    const needs = [text(120), fixed(640), text(260)];
    for (const available of [0, 500, 1020, 1500]) {
      const widths = fitWidths(needs, available);
      widths.forEach((width, index) =>
        expect(
          width,
          `available ${available}, column ${index}`
        ).toBeGreaterThanOrEqual(needs[index]!.natural)
      );
    }
  });
});
