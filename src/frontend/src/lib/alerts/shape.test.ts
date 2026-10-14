import { describe, expect, it } from "vitest";

import type { DatasetDeclaration, MetricDefinition } from "@/api/custom-types";

import { columnKinds, isRatio, metricShape } from "./shape";

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

describe("columnKinds", () => {
  it("types a table metric's columns by what they compute, then by their own type", () => {
    const definition: MetricDefinition = {
      table: "gold.prs",
      fields: [
        { column: "id", type: "string", agg: "count", as_name: "total" },
        { column: "age", type: "int", agg: "avg", as_name: "mean_age" },
        { column: "repo", type: "string", as_name: "repo" },
        { column: "opened", type: "string", agg: "max", as_name: "latest" },
        { column: "size", type: "float", agg: "min", as_name: "smallest" },
        { type: "float", as_name: "share", divide: ["total", "smallest"] },
      ],
    };

    expect(columnKinds(definition)).toEqual([
      ["total", "number"],
      ["mean_age", "number"],
      ["repo", "text"],
      ["latest", "text"],
      ["smallest", "number"],
      ["share", "number"],
    ]);
  });

  it("types a dataset metric's plain columns by what the dataset declares", () => {
    const declaration: DatasetDeclaration = {
      title: "Runs",
      fields: [
        { name: "stand", path: "stand", type: "string" },
        { name: "green", path: "green", type: "bool" },
        { name: "started", path: "started", type: "datetime" },
        { name: "ms", path: "ms", type: "int" },
      ],
    };
    const definition = metric({
      fields: [
        STAND,
        { field: "green", type: "string", as_name: "green" },
        { field: "started", type: "string", agg: "max", as_name: "last_run" },
        { field: "ms", type: "string", agg: "max", as_name: "slowest_ms" },
        { field: "unknown", type: "int", as_name: "fallback" },
        PASSED,
      ],
    });

    expect(columnKinds(definition, declaration)).toEqual([
      ["stand", "text"],
      ["green", "text"],
      ["last_run", "date"],
      ["slowest_ms", "number"],
      ["fallback", "number"],
      ["passed", "number"],
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
