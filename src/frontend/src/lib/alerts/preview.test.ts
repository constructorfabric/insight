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

  it.each([
    ["a power of two past 2^53", 2 ** 60, 1],
    ["2^53 itself, which a rounded neighbour also lands on", 2 ** 53, "9007199254740993"],
    ["a wide whole number with no threshold", 2 ** 60, undefined],
  ] as const)(
    "shows %s about, without a verdict, since JSON parsing may have rounded it",
    (_case, value, threshold) => {
      expect(
        previewCheck(answer(["total"], [[value]]), "total", ">=", threshold)
      ).toEqual({ kind: "wide", value });
    }
  );

  it.each([
    [">", 12, "9007199254740993", false],
    ["<", 12, "9007199254740993", true],
    [">=", 9007199254740991, "9007199254740991", true],
    [">", 9007199254740991, "9007199254740993", false],
    ["<", -5, "-9007199254740993", false],
    [">", 0.5, "9007199254740992", false],
  ] as const)(
    "reads %s with value %s against the stored digits %s exactly as breached: %s",
    (operator, value, threshold, breached) => {
      expect(
        previewCheck(answer(["total"], [[value]]), "total", operator, threshold),
        `${value} ${operator} ${threshold}`
      ).toEqual({ kind: "value", value, breached });
    }
  );

  it.each([
    ["a fraction against stored digits past 2^53", 0.5, "9007199254740993"],
    ["a fraction against stored digits that are not a whole number", 0.5, "0.5"],
  ])("is unknown for %s", (_case, value, threshold) => {
    expect(previewCheck(answer(["total"], [[value]]), "total", ">", threshold)).toEqual({
      kind: "unknown",
      reason: "incomparable",
    });
  });

  it("reads the value alone while no threshold is typed", () => {
    expect(
      previewCheck(answer(["total"], [[12]]), "total", ">", undefined)
    ).toEqual({ kind: "value", value: 12, breached: undefined });
  });
});
