import { describe, expect, it } from "vitest";

import type { MetricResult } from "@/api/custom-client";

import { bridgeRows } from "./bridge";

function result(rows: unknown[][]): MetricResult {
  return { columns: ["step", "delta"], rows };
}

describe("bridgeRows", () => {
  it("floats each delta from the running total and closes on the total", () => {
    const rows = bridgeRows(
      result([
        ["start", 100],
        ["gain", 30],
        ["loss", -50],
      ]),
      "step",
      "delta"
    );

    expect(rows).toEqual([
      { label: "start", low: 0, high: 100, delta: 100, kind: "increase" },
      { label: "gain", low: 100, high: 130, delta: 30, kind: "increase" },
      { label: "loss", low: 80, high: 130, delta: -50, kind: "decrease" },
      { label: "Total", low: 0, high: 80, delta: 80, kind: "total" },
    ]);
  });

  it("draws one delta and its total from a single row", () => {
    expect(bridgeRows(result([["only", 7]]), "step", "delta")).toEqual([
      { label: "only", low: 0, high: 7, delta: 7, kind: "increase" },
      { label: "Total", low: 0, high: 7, delta: 7, kind: "total" },
    ]);
  });

  it("hangs a negative total below zero", () => {
    const rows = bridgeRows(result([["loss", -4]]), "step", "delta");

    expect(rows.at(-1)).toEqual({
      label: "Total",
      low: -4,
      high: 0,
      delta: -4,
      kind: "total",
    });
  });

  it("skips a row whose delta is not a number", () => {
    const rows = bridgeRows(
      result([
        ["a", 2],
        ["b", null],
      ]),
      "step",
      "delta"
    );

    expect(rows.map((row) => row.label)).toEqual(["a", "Total"]);
  });

  it("draws nothing for no rows", () => {
    expect(bridgeRows(result([]), "step", "delta")).toEqual([]);
  });
});
