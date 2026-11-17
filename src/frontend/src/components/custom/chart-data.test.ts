import { describe, expect, it, vi } from "vitest";
import type { MouseHandlerDataParam } from "recharts";

import { pointClick, stackColor, stacks } from "./chart-data";

const at = (activeTooltipIndex: unknown) =>
  ({ activeTooltipIndex }) as MouseHandlerDataParam;

describe("pointClick", () => {
  it.each([
    ["a numeric index", 1, 1],
    ["a string index", "1", 1],
    ["no point under the pointer", null, 0],
    ["an index Recharts left out", undefined, 0],
  ])("reports %s accordingly", (_case, index, calls) => {
    const onPointClick = vi.fn();
    pointClick(onPointClick)?.(at(index));
    expect(onPointClick, _case).toHaveBeenCalledTimes(calls);
  });

  it("gives a chart no click handler when nothing listens", () => {
    expect(pointClick(undefined)).toBeUndefined();
  });
});

describe("stacks", () => {
  const runs = {
    columns: ["week", "outcome", "runs"],
    rows: [
      ["2026-11-02", "merged", 3],
      ["2026-11-02", "failed", "1"],
      ["2026-11-09", "merged", 2],
    ],
  };

  it("draws one bar per x in row order, split by the series sorted by name", () => {
    const { rows, series } = stacks(runs, "week", "runs", "outcome");

    expect(series.map((segment) => segment.label)).toEqual([
      "failed",
      "merged",
    ]);
    expect(rows.map((row) => row.week)).toEqual(["2026-11-02", "2026-11-09"]);
  });

  it("puts each row's value in its series' segment, and nothing where a series is absent", () => {
    const { rows, series } = stacks(runs, "week", "runs", "outcome");
    const [failed, merged] = series.map((segment) => segment.key);

    expect(rows[0][merged]).toBe(3);
    expect(rows[0][failed]).toBe(1);
    expect(rows[1][merged]).toBe(2);
    expect(rows[1][failed]).toBe(0);
  });

  it("totals each bar across its series", () => {
    const { rows, total } = stacks(runs, "week", "runs", "outcome");

    expect(rows.map((row) => row[total])).toEqual([4, 2]);
  });

  it("keys segments so a value with dots or spaces cannot be read as a path", () => {
    const { series } = stacks(
      {
        columns: ["week", "cause", "runs"],
        rows: [["2026-11-02", "build.timed out", 1]],
      },
      "week",
      "runs",
      "cause"
    );

    expect(series[0].label).toBe("build.timed out");
    expect(series[0].key).not.toMatch(/[.\s]/);
  });

  it("keeps an x column whose name looks like a key it adds", () => {
    const { rows, total } = stacks(
      {
        columns: ["stack_total", "outcome", "runs"],
        rows: [
          ["2026-11-02", "merged", 3],
          ["2026-11-02", "failed", 1],
        ],
      },
      "stack_total",
      "runs",
      "outcome"
    );

    expect(rows[0].stack_total).toBe("2026-11-02");
    expect(rows[0][total]).toBe(4);
  });
});

describe("stacks' top segment", () => {
  it("is the highest segment holding a value, so a total sits on the visible top", () => {
    const { rows, series, top } = stacks(
      {
        columns: ["week", "result", "mrs"],
        rows: [
          ["2026-11-02", "merged", 4],
          ["2026-11-02", "open", 1],
          ["2026-11-09", "merged", 2],
        ],
      },
      "week",
      "mrs",
      "result"
    );
    const [merged, open] = series.map((segment) => segment.key);

    expect(rows.map((row) => row[top])).toEqual([open, merged]);
  });
});

describe("stackColor", () => {
  it("paints twelve segments apart before it repeats", () => {
    const colors = Array.from({ length: 13 }, (_, index) => stackColor(index));

    expect(new Set(colors.slice(0, 12)).size).toBe(12);
    expect(colors[12]).toBe(colors[0]);
  });
});
