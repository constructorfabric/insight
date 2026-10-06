import { describe, expect, it } from "vitest";

import type { StoredMetric } from "@/api/custom-types";

import { takesWindow, windowFor } from "./window";

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

describe("takesWindow", () => {
  it.each([
    ["a dated metric", DATED, true],
    ["an undated metric", UNDATED, false],
    ["a metric not read yet", undefined, true],
  ])("for %s", (_case, stored, open) => {
    expect(takesWindow(stored), _case).toBe(open);
  });
});

describe("windowFor", () => {
  it.each([
    ["a dated metric with no window yet", DATED, "", "P7D"],
    ["a dated metric with a window chosen", DATED, "P30D", "P30D"],
    ["an undated metric", UNDATED, "P7D", ""],
    ["a metric not read yet", undefined, "", ""],
    ["a metric not read yet, with a window chosen", undefined, "P30D", "P30D"],
  ])("moves %s to the right window", (_case, stored, current, next) => {
    expect(windowFor(stored, current), _case).toBe(next);
  });
});
