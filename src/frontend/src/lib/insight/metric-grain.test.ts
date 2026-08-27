import { describe, expect, it } from "vitest";

import type { MetricResult } from "@/api/metric-results-client";
import { normalizeMetricResults } from "@/lib/metrics/collection";
import {
  activityEvents,
  dailyReadings,
  finestGrain,
} from "@/lib/insight/metric-grain";

function metric(granularity?: string[]): MetricResult | undefined {
  return {
    metric_key: "git.commits",
    label: "Commits",
    unit: null,
    format: "integer",
    computation: "sum",
    direction: "higher_is_better",
    views: [],
    ...(granularity ? { drilldown: { granularity } } : {}),
  } as unknown as MetricResult;
}

function normalized(granularity?: string[]) {
  return normalizeMetricResults([metric(granularity)!]).get("git.commits");
}

const DAY_COLUMNS = [
  { key: "date", label: "Date", type: "date" as const },
  { key: "value", label: "Value", type: "number" as const },
];
const RATIO_COLUMNS = [
  { key: "date", label: "Date", type: "date" as const },
  { key: "numerator", label: "Numerator", type: "number" as const },
  { key: "denominator", label: "Denominator", type: "number" as const },
];

function rows(values: Record<string, unknown>[]) {
  return values.map((v) => ({ values: v }));
}

describe("finestGrain", () => {
  it("takes the closest look a metric offers", () => {
    expect(finestGrain(normalized(["event", "source_summary"]))).toBe("event");
    expect(finestGrain(normalized(["source_summary"]))).toBe("source_summary");
    expect(finestGrain(normalized(["derived_population"]))).toBe(
      "derived_population"
    );
  });

  it("says a metric offers none rather than guessing one", () => {
    // A metric with no declared detail must render as having none. Falling
    // back to a default would draw a daily picture for something the API
    // cannot break down at all.
    expect(finestGrain(normalized())).toBeNull();
    expect(finestGrain(undefined)).toBeNull();
    expect(finestGrain(normalized([]))).toBeNull();
  });
});

describe("dailyReadings", () => {
  it("adds up the rows a day was split across", () => {
    // The wire splits a day per dimension and returns the dimension only when
    // asked for it, so unsummed rows read as duplicates of the same date.
    expect(
      dailyReadings(
        rows([
          { date: "2026-03-02", value: 30 },
          { date: "2026-03-02", value: 12 },
          { date: "2026-03-01", value: 5 },
        ]),
        DAY_COLUMNS
      )
    ).toEqual([
      { date: "2026-03-01", value: 5, numerator: null, denominator: null },
      { date: "2026-03-02", value: 42, numerator: null, denominator: null },
    ]);
  });

  it("divides a ratio once, on summed sides", () => {
    // Averaging daily shares would weight a day holding one meeting the same
    // as a day holding eight.
    const days = dailyReadings(
      rows([
        { date: "2026-03-01", numerator: 2, denominator: 8 },
        { date: "2026-03-01", numerator: 2, denominator: 0 },
      ]),
      RATIO_COLUMNS
    );
    expect(days).toEqual([
      { date: "2026-03-01", value: 0.5, numerator: 4, denominator: 8 },
    ]);
  });

  it("leaves a day with no denominator alone rather than dividing by zero", () => {
    const days = dailyReadings(
      rows([{ date: "2026-03-01", numerator: 3, denominator: 0 }]),
      RATIO_COLUMNS
    );
    expect(days[0]?.value).toBe(0);
    expect(Number.isFinite(days[0]?.value ?? NaN)).toBe(true);
  });

  it("drops a row that names no day", () => {
    expect(dailyReadings(rows([{ value: 4 }]), DAY_COLUMNS)).toEqual([]);
  });
});

describe("activityEvents", () => {
  it("keeps the subject of a title and leaves the body behind", () => {
    // A commit message carries its reasoning after the first line; in a list
    // being scanned that reasoning is noise, and in the export it is intact.
    const events = activityEvents([
      {
        values: {
          date: "2026-03-01",
          ref: "abc123",
          title: "Fix the thing\n\nThe long why, several paragraphs of it.\n",
          repository: "example/app",
        },
        links: { title: "https://git.example/example/app/commit/abc123" },
      },
    ]);
    expect(events[0]?.title).toBe("Fix the thing");
    expect(events[0]?.context).toBe("example/app");
    expect(events[0]?.ref).toBe("abc123");
    expect(events[0]?.links).toEqual({
      title: "https://git.example/example/app/commit/abc123",
    });
  });

  it("puts the newest first — a section is read from now backwards", () => {
    const events = activityEvents(
      rows([
        { date: "2026-03-01", title: "older" },
        { date: "2026-03-09", title: "newest" },
        { date: "2026-03-05", title: "middle" },
      ])
    );
    expect(events.map((e) => e.title)).toEqual(["newest", "middle", "older"]);
  });

  it("survives a source that reports no title, ref, or place", () => {
    // Not every event-grade source names its things; the row still happened
    // and must not be dropped for missing a label.
    const events = activityEvents(rows([{ date: "2026-03-01", value: 3 }]));
    expect(events).toEqual([
      {
        date: "2026-03-01",
        ref: null,
        title: null,
        context: null,
        value: 3,
        links: {},
        values: { date: "2026-03-01", value: 3 },
      },
    ]);
  });
});
