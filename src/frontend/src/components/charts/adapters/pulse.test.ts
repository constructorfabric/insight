import { describe, expect, it } from "vitest";

import type { MetricResult } from "@/api/custom-client";

import { pulseSummary } from "./pulse";

function result(rows: unknown[][]): MetricResult {
  return { columns: ["day", "n"], rows };
}

describe("pulseSummary", () => {
  it("reads the last value and its change from the first", () => {
    const summary = pulseSummary(
      result([
        ["d1", 100],
        ["d2", null],
        ["d3", 125],
      ]),
      "day",
      "n"
    );

    expect(summary).toEqual({
      latest: 125,
      change: 25,
      points: [100, null, 125],
      since: "d1",
      until: "d3",
    });
  });

  it("has no change from a single reading", () => {
    expect(pulseSummary(result([["d1", 7]]), "day", "n")).toEqual({
      latest: 7,
      change: null,
      points: [7],
      since: "d1",
      until: "d1",
    });
  });

  it("has no change from a first reading of zero", () => {
    expect(
      pulseSummary(
        result([
          ["d1", 0],
          ["d2", 5],
        ]),
        "day",
        "n"
      ).change
    ).toBeNull();
  });

  it("has no latest value when nothing was read", () => {
    expect(pulseSummary(result([["d1", null]]), "day", "n").latest).toBeNull();
  });
});
