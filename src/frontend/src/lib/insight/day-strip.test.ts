import { describe, expect, it } from "vitest";

import {
  beforeAvailableDataDays,
  notYetCollectedDays,
  provisionalDays,
  silentDays,
  stripDays,
} from "@/lib/insight/day-strip";
import type { DayReading } from "@/lib/insight/metric-grain";
import type { CollectionBoundary } from "@/queries/metric-definitions";

function reading(date: string, value: number): DayReading {
  return { date, value, numerator: null, denominator: null };
}

function boundary(over: Partial<CollectionBoundary> = {}): CollectionBoundary {
  return {
    collectedFrom: null,
    collectedThrough: null,
    settledThrough: null,
    ...over,
  };
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
      [
        reading("2026-02-14", 100),
        reading("2026-03-01", 5),
        reading("2026-03-02", 10),
      ],
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
      [
        {
          date: "2026-03-01",
          value: 0.5,
          numerator: 4,
          denominator: 8,
        },
      ],
      "2026-03-01",
      "2026-03-01"
    );
    expect(days[0]).toMatchObject({ numerator: 4, denominator: 8 });
  });

  it("tells a day older than the data from one ahead of it", () => {
    // Neither is a day the metric measured as empty, and they are not the same
    // finding — a reader can ask for backfill on the first and only wait for
    // the second, so the strip must not fold them together.
    const days = stripDays(
      [reading("2026-03-03", 1)],
      "2026-03-01",
      "2026-03-05",
      boundary({ collectedFrom: "2026-03-02", collectedThrough: "2026-03-04" })
    );
    expect(days.map((d) => d.coverage)).toEqual([
      "before_available_data",
      "covered",
      "covered",
      "covered",
      "not_yet_collected",
    ]);
    expect(beforeAvailableDataDays(days)).toBe(1);
    expect(notYetCollectedDays(days)).toBe(1);
    // Only the days inside the pair can be silent: the two outside are not
    // days the metric holds a reading for either way.
    expect(silentDays(days)).toBe(2);
  });

  it("leaves an absent bound open rather than calling its side uncovered", () => {
    const days = stripDays(
      [reading("2026-03-01", 1)],
      "2026-03-01",
      "2026-03-03",
      boundary({ collectedThrough: "2026-03-02" })
    );
    expect(days.map((d) => d.coverage)).toEqual([
      "covered",
      "covered",
      "not_yet_collected",
    ]);
    expect(beforeAvailableDataDays(days)).toBe(0);
  });

  it("marks every delivered day after the settled boundary and none before", () => {
    const days = stripDays(
      [reading("2026-03-01", 1), reading("2026-03-03", 2)],
      "2026-03-01",
      "2026-03-04",
      boundary({ collectedThrough: "2026-03-03", settledThrough: "2026-03-02" })
    );
    expect(days.map((d) => d.provisional)).toEqual([false, false, true, false]);
    expect(provisionalDays(days)).toBe(1);
    // A day the source has not delivered is not open to revision — there is
    // nothing there to revise.
    expect(days[3]?.coverage).toBe("not_yet_collected");
  });

  it("settles everything when the catalogue declares no boundary", () => {
    const days = stripDays(
      [reading("2026-03-01", 1)],
      "2026-03-01",
      "2026-03-02",
      boundary({ collectedThrough: "2026-03-02" })
    );
    expect(provisionalDays(days)).toBe(0);
  });

  it("keeps a historical period free of provisional days", () => {
    // A window that ends before the settled boundary holds nothing revisable.
    // Deriving the boundary from a day count instead marked the whole window,
    // because the count was measured from the newest delivery, not from the
    // window being drawn.
    const days = stripDays(
      [reading("2026-02-10", 1)],
      "2026-02-01",
      "2026-02-28",
      boundary({
        collectedFrom: "2026-01-01",
        collectedThrough: "2026-03-10",
        settledThrough: "2026-02-28",
      })
    );
    expect(provisionalDays(days)).toBe(0);
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
