import { describe, expect, it } from "vitest";

import type { MetricResult } from "@/api/custom-client";

import { calendarCells, calendarGrain } from "./calendar";

function result(rows: unknown[][]): MetricResult {
  return { columns: ["day", "n"], rows };
}

describe("calendarCells", () => {
  it("fills every day between the first and the last with a cell", () => {
    const cells = calendarCells(
      result([
        ["2026-09-01 00:00:00.000", 2],
        ["2026-09-03", 4],
      ]),
      "day",
      "n"
    );

    expect(cells.map((cell) => [cell.date, cell.value])).toEqual([
      ["2026-09-01", 2],
      ["2026-09-02", 0],
      ["2026-09-03", 4],
    ]);
  });

  it("places each day in its week column and weekday row", () => {
    const cells = calendarCells(
      result([
        ["2026-09-05", 1],
        ["2026-09-06", 1],
      ]),
      "day",
      "n"
    );

    expect(cells.map((cell) => [cell.week, cell.weekday])).toEqual([
      [0, 6],
      [1, 0],
    ]);
  });

  it("grades intensity in four steps against the busiest day, empty days at zero", () => {
    const cells = calendarCells(
      result([
        ["2026-09-01", 0],
        ["2026-09-02", 1],
        ["2026-09-03", 2],
        ["2026-09-04", 3],
        ["2026-09-05", 8],
      ]),
      "day",
      "n"
    );

    expect(cells.map((cell) => cell.level)).toEqual([0, 1, 1, 2, 4]);
  });

  it("sums rows of the same day and ignores rows that are not dated", () => {
    const cells = calendarCells(
      result([
        ["2026-09-01", 1],
        ["2026-09-01", 2],
        ["someday", 9],
      ]),
      "day",
      "n"
    );

    expect(cells.map((cell) => [cell.date, cell.value])).toEqual([
      ["2026-09-01", 3],
    ]);
  });

  it("keeps only the last 53 weeks of a longer span", () => {
    const cells = calendarCells(
      result([
        ["2024-01-01", 1],
        ["2026-01-01", 1],
      ]),
      "day",
      "n"
    );

    expect(cells.at(-1)?.date).toBe("2026-01-01");
    expect(Math.max(...cells.map((cell) => cell.week))).toBeLessThanOrEqual(52);
  });
});

describe("calendarGrain", () => {
  it.each([
    ["one row per day", ["2026-09-01", "2026-09-02", "2026-09-04"], "day"],
    ["weekly buckets", ["2026-08-31", "2026-09-07", "2026-09-14"], "week"],
    ["monthly buckets", ["2026-07-01", "2026-08-01", "2026-09-01"], "month"],
    ["a single day", ["2026-09-01"], "day"],
  ] as const)("reads %s", (_name, days, grain) => {
    expect(calendarGrain(result(days.map((day) => [day, 1])), "day")).toBe(
      grain
    );
  });
});
