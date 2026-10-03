import { describe, expect, it } from "vitest";

import type { StoredMetric } from "@/api/custom-types";

import { DEFAULT_WINDOW, windowFor, windowRule } from "./window";

const FIELDS = [
  { field: "runs", type: "int", agg: "sum", as_name: "runs" },
  { field: "id", type: "string", agg: "count", as_name: "cases" },
  { field: "ms", type: "int", agg: "max", as_name: "slowest" },
  {
    type: "float",
    as_name: "rate",
    divide: ["runs", "cases"] as [string, string],
  },
];

const DATED: StoredMetric = {
  definition: { dataset: "runs", fields: FIELDS },
  clock: { field: "date", from: "dataset" },
};
const UNDATED: StoredMetric = {
  definition: { dataset: "runs", fields: FIELDS },
};

describe("windowRule", () => {
  it.each([
    [
      "a sum on a dated metric",
      DATED,
      "runs",
      { allTime: false, windowed: true },
    ],
    [
      "a count on a dated metric",
      DATED,
      "cases",
      { allTime: false, windowed: true },
    ],
    [
      "a maximum on a dated metric",
      DATED,
      "slowest",
      { allTime: true, windowed: true },
    ],
    [
      "a ratio on a dated metric",
      DATED,
      "rate",
      { allTime: true, windowed: true },
    ],
    [
      "any column of an undated metric",
      UNDATED,
      "runs",
      { allTime: true, windowed: false },
    ],
    [
      "a metric not read yet",
      undefined,
      "runs",
      { allTime: true, windowed: true },
    ],
  ])("for %s", (_case, stored, column, open) => {
    const rule = windowRule(stored, column);

    expect({ allTime: rule.allTime, windowed: rule.windowed }).toEqual(open);
  });
});

describe("windowFor", () => {
  it.each([
    ["a dated metric with no window yet", DATED, "slowest", "", DEFAULT_WINDOW],
    ["a dated metric with a window chosen", DATED, "runs", "P30D", "P30D"],
    ["an undated metric", UNDATED, "runs", "P7D", ""],
    ["a metric not read yet", undefined, "runs", "", ""],
  ])("moves %s to the right window", (_case, stored, column, current, next) => {
    expect(windowFor(stored, column, current)).toBe(next);
  });
});
