import { describe, expect, it } from "vitest";

import type { MetricResult } from "@/api/custom-types";

import { previewCheck } from "./preview";

function answer(columns: string[], rows: unknown[][]): MetricResult {
  return { columns, rows, percents: [] } as unknown as MetricResult;
}

describe("previewCheck", () => {
  it.each([
    ["no rows", answer(["total"], []), "no_rows"],
    ["many rows", answer(["total"], [[1], [2]]), "many_rows"],
    ["a missing column", answer(["other"], [[1]]), "column_missing"],
    ["a missing column with no rows", answer(["other"], []), "column_missing"],
    ["an empty value", answer(["total"], [[null]]), "null"],
    ["a numeric string", answer(["total"], [["12"]]), "non_numeric"],
    ["a word", answer(["total"], [["twelve"]]), "non_numeric"],
    [
      "a wide integer against a fraction",
      answer(["total"], [[2 ** 60]]),
      "incomparable",
    ],
  ])("is unknown for %s, as the check would be", (_case, result, reason) => {
    expect(previewCheck(result, "total", ">", 0.5)).toEqual({
      kind: "unknown",
      reason,
    });
  });

  it.each([
    [">", 11, 10, true],
    [">", 10, 10, false],
    [">=", 10, 10, true],
    ["<", 9, 10, true],
    ["<", 10, 10, false],
    ["<=", 10, 10, true],
    [">", 0.75, 0.5, true],
  ] as const)(
    "reads %s with value %s against %s as breached: %s",
    (operator, value, threshold, breached) => {
      expect(
        previewCheck(
          answer(["total"], [[value]]),
          "total",
          operator,
          threshold
        ),
        `${value} ${operator} ${threshold}`
      ).toEqual({ kind: "value", value, breached });
    }
  );

  it("compares a wide integer with a whole threshold", () => {
    expect(
      previewCheck(answer(["total"], [[2 ** 60]]), "total", ">", 1)
    ).toEqual({ kind: "value", value: 2 ** 60, breached: true });
  });
});
