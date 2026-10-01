import { describe, expect, it } from "vitest";

import type { MetricResult } from "@/api/custom-client";

import { categoryRows, progress, radarRows } from "./category";

function result(columns: string[], rows: unknown[][]): MetricResult {
  return { columns, rows };
}

describe("categoryRows", () => {
  it("sums rows that share a label and keeps first-appearance order", () => {
    const rows = categoryRows(
      result(
        ["repo", "n"],
        [
          ["api", 1],
          ["web", 2],
          ["api", 3],
        ]
      ),
      "repo",
      "n"
    );

    expect(rows).toEqual([
      { label: "api", value: 4 },
      { label: "web", value: 2 },
    ]);
  });

  it("drops a row whose value is not a number", () => {
    const rows = categoryRows(
      result(
        ["repo", "n"],
        [
          ["api", null],
          ["web", "x"],
          ["docs", 2],
        ]
      ),
      "repo",
      "n"
    );

    expect(rows).toEqual([{ label: "docs", value: 2 }]);
  });

  it("drops zero and negative shares when only a positive part can be drawn", () => {
    const rows = categoryRows(
      result(
        ["repo", "n"],
        [
          ["api", -2],
          ["web", 0],
          ["docs", 5],
        ]
      ),
      "repo",
      "n",
      { positiveOnly: true }
    );

    expect(rows).toEqual([{ label: "docs", value: 5 }]);
  });

  it("ranks largest first and cuts to the limit", () => {
    const rows = categoryRows(
      result(
        ["who", "n"],
        [
          ["a", 1],
          ["b", 3],
          ["c", 2],
        ]
      ),
      "who",
      "n",
      { order: "desc", limit: 2 }
    );

    expect(rows).toEqual([
      { label: "b", value: 3 },
      { label: "c", value: 2 },
    ]);
  });

  it("keeps the largest shares and sums the rest into Other", () => {
    const rows = categoryRows(
      result(
        ["repo", "n"],
        [
          ["a", 1],
          ["b", 5],
          ["c", 2],
          ["d", 4],
        ]
      ),
      "repo",
      "n",
      { keep: 2 }
    );

    expect(rows).toEqual([
      { label: "b", value: 5 },
      { label: "d", value: 4 },
      { label: "Other", value: 3 },
    ]);
  });

  it("names a missing label (none)", () => {
    expect(categoryRows(result(["who", "n"], [[null, 1]]), "who", "n")).toEqual(
      [{ label: "(none)", value: 1 }]
    );
  });
});

describe("radarRows", () => {
  it("reads a value and an optional target per label", () => {
    const rows = radarRows(
      result(
        ["area", "score", "goal"],
        [
          ["speed", 60, 80],
          ["quality", 70, null],
        ]
      ),
      "area",
      "score",
      "goal"
    );

    expect(rows).toEqual([
      { label: "speed", value: 60, target: 80 },
      { label: "quality", value: 70, target: null },
    ]);
  });
});

describe("progress", () => {
  it.each([
    ["a share of its maximum", [[30, 120]], "goal", 25],
    ["a value past its maximum, capped", [[150, 120]], "goal", 100],
    ["a bare percentage when no maximum is named", [[42, 0]], undefined, 42],
    ["a negative value, floored", [[-5, 10]], "goal", 0],
    ["a zero maximum as nothing done", [[5, 0]], "goal", 0],
  ])("reads %s", (_name, rows, max, percent) => {
    const read = progress(result(["done", "goal"], rows), "done", max);

    expect(read.percent).toBe(percent);
  });

  it("has no percentage when there is no row", () => {
    expect(progress(result(["done"], []), "done").percent).toBeNull();
  });
});
