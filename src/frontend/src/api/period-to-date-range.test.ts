import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  MAX_DATE_RANGE_DAYS,
  periodToDateRange,
  previousPeriodRange,
  resolveDateRange,
  throughToday,
  toISODate,
} from "./period-to-date-range";

describe("toISODate", () => {
  it("zero-pads the local date", () => {
    expect(toISODate(new Date(2026, 0, 5))).toBe("2026-01-05");
  });
});

describe("periodToDateRange", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("ends yesterday and spans 7 local days for a week", () => {
    vi.setSystemTime(new Date(2026, 5, 15, 10, 30)); // local 2026-06-15
    expect(periodToDateRange("week")).toEqual({
      from: "2026-06-08",
      to: "2026-06-14",
    });
  });

  it("spans one calendar month", () => {
    vi.setSystemTime(new Date(2026, 5, 15, 10, 30));
    expect(periodToDateRange("month")).toEqual({
      from: "2026-05-15",
      to: "2026-06-14",
    });
  });

  it("spans three calendar months", () => {
    vi.setSystemTime(new Date(2026, 5, 15, 10, 30));
    expect(periodToDateRange("quarter")).toEqual({
      from: "2026-03-15",
      to: "2026-06-14",
    });
  });

  it("spans one calendar year", () => {
    vi.setSystemTime(new Date(2026, 5, 15, 10, 30));
    expect(periodToDateRange("year")).toEqual({
      from: "2025-06-15",
      to: "2026-06-14",
    });
  });

  it("clamps month arithmetic at short-month boundaries", () => {
    vi.setSystemTime(new Date(2026, 2, 31, 8, 0)); // local 2026-03-31
    // to = 2026-03-30; one month back clamps 30 → Feb 28, +1 day → Mar 1.
    expect(periodToDateRange("month")).toEqual({
      from: "2026-03-01",
      to: "2026-03-30",
    });
  });
});

describe("resolveDateRange", () => {
  it("prefers a valid custom range over the period", () => {
    expect(
      resolveDateRange("week", { from: "2026-01-01", to: "2026-01-31" }),
    ).toEqual({ from: "2026-01-01", to: "2026-01-31" });
  });

  it("throws on a malformed custom range", () => {
    expect(() =>
      resolveDateRange("week", { from: "01/01/2026", to: "2026-01-31" }),
    ).toThrow(/Invalid date range/);
  });

  it("throws on an inverted custom range", () => {
    expect(() =>
      resolveDateRange("week", { from: "2026-02-01", to: "2026-01-31" }),
    ).toThrow(/Invalid date range/);
  });

  it("falls back to the period when no custom range is set", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 5, 15, 10, 30));
    try {
      expect(resolveDateRange("week", null)).toEqual({
        from: "2026-06-08",
        to: "2026-06-14",
      });
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("previousPeriodRange", () => {
  it("shifts a week range back 7 days", () => {
    expect(
      previousPeriodRange({ from: "2026-06-08", to: "2026-06-14" }, "week"),
    ).toEqual({ from: "2026-06-01", to: "2026-06-07" });
  });

  it("shifts a month range back one month, clamping to short months", () => {
    expect(
      previousPeriodRange({ from: "2026-03-31", to: "2026-04-30" }, "month"),
    ).toEqual({ from: "2026-02-28", to: "2026-03-30" });
  });

  it("shifts a quarter range back three months, clamping day-of-month", () => {
    expect(
      previousPeriodRange({ from: "2026-05-31", to: "2026-08-30" }, "quarter"),
    ).toEqual({ from: "2026-02-28", to: "2026-05-30" });
  });

  it("shifts a year range back one year", () => {
    expect(
      previousPeriodRange({ from: "2025-06-15", to: "2026-06-14" }, "year"),
    ).toEqual({ from: "2024-06-15", to: "2025-06-14" });
  });
});

describe("throughToday", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 5, 15, 10, 30)); // local 2026-06-15
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("extends a range that stops at yesterday", () => {
    expect(throughToday({ from: "2026-06-08", to: "2026-06-14" })).toEqual({
      from: "2026-06-08",
      to: "2026-06-15",
    });
  });

  it("extends every preset, since each one ends yesterday", () => {
    for (const period of ["week", "month", "quarter", "year"] as const) {
      expect(throughToday(periodToDateRange(period)).to).toBe("2026-06-15");
    }
  });

  it("leaves a range already reaching today alone", () => {
    const range = { from: "2026-06-01", to: "2026-06-15" };
    expect(throughToday(range)).toBe(range);
  });

  it("leaves a range deliberately left in the past alone", () => {
    const range = { from: "2026-01-01", to: "2026-01-31" };
    expect(throughToday(range)).toBe(range);
  });

  it("refuses to extend past the request cap", () => {
    // Exactly at the cap and ending yesterday: one more day would be rejected
    // by the API, so the strip keeps the window it can actually ask for.
    const from = new Date(2026, 5, 14);
    from.setDate(from.getDate() - (MAX_DATE_RANGE_DAYS - 1));
    const range = { from: toISODate(from), to: "2026-06-14" };
    expect(throughToday(range)).toBe(range);
  });
});
