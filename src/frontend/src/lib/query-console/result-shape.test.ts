import { describe, expect, it } from "vitest";

import {
  formatCell,
  inferChartModel,
  inferColumns,
  toNumber,
  type ResultRow,
} from "./result-shape";

describe("inferColumns", () => {
  it("returns keys in first-seen order across the row union", () => {
    const rows: ResultRow[] = [
      { a: 1, b: 2 },
      { b: 3, c: 4 },
    ];
    expect(inferColumns(rows)).toEqual(["a", "b", "c"]);
  });

  it("is empty for no rows", () => {
    expect(inferColumns([])).toEqual([]);
  });
});

describe("inferChartModel", () => {
  it("picks the first categorical column as label and numeric columns as series", () => {
    const rows: ResultRow[] = [
      { tool: "github", commits: 10, prs: 3 },
      { tool: "gitlab", commits: 7, prs: 2 },
    ];
    expect(inferChartModel(rows)).toEqual({
      labelKey: "tool",
      valueKeys: ["commits", "prs"],
    });
  });

  it("treats string-encoded numbers (ClickHouse wide ints) as numeric", () => {
    const rows: ResultRow[] = [
      { team: "alpha", total: "1024" },
      { team: "beta", total: "2048" },
    ];
    expect(inferChartModel(rows)).toEqual({
      labelKey: "team",
      valueKeys: ["total"],
    });
  });

  it("is not chartable without a categorical column", () => {
    const rows: ResultRow[] = [{ x: 1, y: 2 }];
    expect(inferChartModel(rows)).toBeNull();
  });

  it("is not chartable without a numeric column", () => {
    const rows: ResultRow[] = [{ name: "a", label: "b" }];
    expect(inferChartModel(rows)).toBeNull();
  });

  it("is not chartable with more than one categorical column (ambiguous axis)", () => {
    const rows: ResultRow[] = [
      { team: "alpha", tool: "github", commits: 10 },
      { team: "beta", tool: "gitlab", commits: 7 },
    ];
    expect(inferChartModel(rows)).toBeNull();
  });

  it("is not chartable for zero rows", () => {
    expect(inferChartModel([])).toBeNull();
  });

  it("is not chartable beyond the row cap", () => {
    const rows: ResultRow[] = Array.from({ length: 51 }, (_, i) => ({
      label: `row-${i}`,
      value: i,
    }));
    expect(inferChartModel(rows)).toBeNull();
  });

  it("keeps a numeric column that has null gaps but at least one value", () => {
    const rows: ResultRow[] = [
      { day: "mon", count: 5 },
      { day: "tue", count: null },
    ];
    expect(inferChartModel(rows)).toEqual({
      labelKey: "day",
      valueKeys: ["count"],
    });
  });

  it("does not treat an all-null column as numeric", () => {
    const rows: ResultRow[] = [
      { day: "mon", note: null },
      { day: "tue", note: null },
    ];
    expect(inferChartModel(rows)).toBeNull();
  });
});

describe("toNumber", () => {
  it("passes finite numbers, coerces numeric strings, rejects the rest", () => {
    expect(toNumber(42)).toBe(42);
    expect(toNumber("3.5")).toBe(3.5);
    expect(toNumber("")).toBeNull();
    expect(toNumber("abc")).toBeNull();
    expect(toNumber(null)).toBeNull();
    expect(toNumber(Infinity)).toBeNull();
  });
});

describe("formatCell", () => {
  it("renders nulls as a dash, objects as JSON, scalars as strings", () => {
    expect(formatCell(null)).toBe("—");
    expect(formatCell(undefined)).toBe("—");
    expect(formatCell(7)).toBe("7");
    expect(formatCell("hi")).toBe("hi");
    expect(formatCell({ a: 1 })).toBe('{"a":1}');
  });
});
