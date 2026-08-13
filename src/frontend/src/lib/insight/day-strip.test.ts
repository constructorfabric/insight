import { describe, expect, it } from "vitest";

import { silentDays, stripDays } from "@/lib/insight/day-strip";
import type { DayReading } from "@/lib/insight/metric-grain";

function reading(date: string, value: number): DayReading {
  return { date, value, numerator: null, denominator: null };
}

describe("stripDays", () => {
  it("walks the calendar, not the rows", () => {
    // Built from rows alone, four busy days in a month would pack together
    // and read as a month that was busy throughout.
    const days = stripDays(
      [reading("2026-03-01", 4), reading("2026-03-05", 2)],
      "2026-03-01",
      "2026-03-05"
    );
    expect(days).toHaveLength(5);
    expect(days.map((d) => d.value)).toEqual([4, null, null, null, 2]);
  });

  it("keeps a measured zero apart from a day with no reading", () => {
    // They draw the same and mean opposite things: silence from the source
    // versus a day this person did none of it.
    const days = stripDays(
      [reading("2026-03-01", 0)],
      "2026-03-01",
      "2026-03-02"
    );
    expect(days[0]?.value).toBe(0);
    expect(days[0]?.height).toBe(0);
    expect(days[1]?.value).toBeNull();
    expect(days[1]?.height).toBeNull();
  });

  it("scales to the tallest day of the period", () => {
    const days = stripDays(
      [reading("2026-03-01", 5), reading("2026-03-02", 10)],
      "2026-03-01",
      "2026-03-02"
    );
    expect(days.map((d) => d.height)).toEqual([0.5, 1]);
  });

  it("survives a period where nothing happened at all", () => {
    const days = stripDays(
      [reading("2026-03-01", 0), reading("2026-03-02", 0)],
      "2026-03-01",
      "2026-03-02"
    );
    expect(days.every((d) => d.height === 0)).toBe(true);
  });

  it("scales to the tallest day on the strip, not the tallest reading given", () => {
    // A reading outside the window sets the height of nothing it is drawn
    // beside; letting it into the scale makes every visible bar read short.
    const days = stripDays(
      [reading("2026-02-14", 100), reading("2026-03-01", 5), reading("2026-03-02", 10)],
      "2026-03-01",
      "2026-03-02"
    );
    expect(days.map((d) => d.height)).toEqual([0.5, 1]);
  });

  it("refuses a range it cannot walk", () => {
    expect(stripDays([], "2026-03-05", "2026-03-01")).toEqual([]);
    expect(stripDays([], "not-a-date", "2026-03-01")).toEqual([]);
  });

  it("carries both sides of a ratio through to the day", () => {
    const days = stripDays(
      [{ date: "2026-03-01", value: 0.5, numerator: 4, denominator: 8 }],
      "2026-03-01",
      "2026-03-01"
    );
    expect(days[0]).toMatchObject({ numerator: 4, denominator: 8 });
  });
});

describe("silentDays", () => {
  it("counts the days the source said nothing about", () => {
    const days = stripDays(
      [reading("2026-03-01", 1)],
      "2026-03-01",
      "2026-03-04"
    );
    expect(silentDays(days)).toBe(3);
  });
});
