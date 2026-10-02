import { describe, expect, it } from "vitest";

import type { MetricDefinition } from "@/api/custom-types";

import { alertColumns, isRatio, metricShape } from "./shape";

const PASSED = { field: "passed", type: "int", agg: "sum", as_name: "passed" };
const RUNS = { field: "runs", type: "int", agg: "sum", as_name: "runs" };
const RATE = {
  type: "float",
  as_name: "pass_rate",
  divide: ["passed", "runs"] as [string, string],
};
const STAND = { field: "stand", type: "string", as_name: "stand" };

function metric(change: Partial<MetricDefinition>): MetricDefinition {
  return { dataset: "runs", fields: [PASSED, RUNS, RATE], ...change };
}

describe("metricShape", () => {
  it.each([
    ["aggregates with no grouping", metric({}), { kind: "one" }],
    [
      "a grouping",
      metric({ fields: [STAND, PASSED], group_by: ["stand"] }),
      { kind: "grouped", by: ["stand"] },
    ],
    [
      "a grouping cut to its top row",
      metric({ fields: [STAND, PASSED], group_by: ["stand"], limit: 1 }),
      { kind: "one" },
    ],
    [
      "a plain column and no grouping",
      metric({ fields: [STAND, PASSED] }),
      { kind: "rows" },
    ],
  ])("reads %s", (_case, definition, shape) => {
    expect(metricShape(definition)).toEqual(shape);
  });
});

describe("alertColumns", () => {
  it("offers numbers, never what the rows are grouped by", () => {
    const definition = metric({
      fields: [
        STAND,
        PASSED,
        RATE,
        { field: "when", type: "string", agg: "max", as_name: "last_at" },
        { field: "ms", type: "int", agg: "max", as_name: "slowest_ms" },
        { field: "id", type: "string", agg: "count", as_name: "cases" },
      ],
      group_by: ["stand"],
    });

    expect(alertColumns(definition).map((field) => field.as_name)).toEqual([
      "passed",
      "pass_rate",
      "slowest_ms",
      "cases",
    ]);
  });
});

describe("isRatio", () => {
  it.each([
    ["pass_rate", true],
    ["passed", false],
    ["missing", false],
  ])("says whether %s is a ratio", (column, ratio) => {
    expect(isRatio(metric({}), column)).toBe(ratio);
  });

  it("knows nothing before the definition is read", () => {
    expect(isRatio(undefined, "pass_rate")).toBe(false);
  });
});
