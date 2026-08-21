import { describe, expect, it } from "vitest";

import {
  NO_METRIC_VALUE,
  formatMetricNumber,
  formatMetricValue,
  formatPp,
  formatUtcAge,
  formatUtcClock,
  formatUtcInstant,
  metricDisplayUnit,
} from "@/lib/format";

describe("formatMetricNumber / formatMetricValue / metricDisplayUnit", () => {
  it("formats currency without fraction digits", () => {
    expect(formatMetricNumber(1234, "currency")).toBe("$1,234");
    expect(formatMetricValue(1234, "currency", "USD")).toBe("$1,234");
  });

  it("rounds decimal format to one decimal and integers to whole", () => {
    expect(formatMetricNumber(1.24, "decimal")).toBe("1.2");
    expect(formatMetricNumber(1234.6, "integer")).toBe("1,235");
  });

  it("suffixes percent and unit forms", () => {
    expect(formatMetricValue(42.4, "percent")).toBe("42%");
    expect(formatMetricValue(5, "integer", "h")).toBe("5 h");
    expect(formatMetricValue(5, "integer", null)).toBe("5");
  });

  it("renders no-data as an em-dash, never a fabricated zero", () => {
    // A null/undefined/non-finite metric must read as "no data", never 0.
    for (const empty of [null, undefined, NaN, Infinity, -Infinity] as const) {
      expect(formatMetricNumber(empty, "integer")).toBe(NO_METRIC_VALUE);
      expect(formatMetricValue(empty, "currency", "USD")).toBe(NO_METRIC_VALUE);
      expect(formatMetricValue(empty, "percent")).toBe(NO_METRIC_VALUE);
      expect(formatMetricValue(empty, "integer", "h")).toBe(NO_METRIC_VALUE);
    }
    // A real zero is still a real value and must format as such.
    expect(formatMetricNumber(0, "integer")).toBe("0");
    expect(formatMetricValue(0, "percent")).toBe("0%");
  });

  it("hides the side unit when the number already carries it", () => {
    expect(metricDisplayUnit("currency", "USD")).toBeUndefined();
    expect(metricDisplayUnit("percent", "%")).toBeUndefined();
    expect(metricDisplayUnit("integer", "h")).toBe("h");
    expect(metricDisplayUnit("integer", null)).toBeUndefined();
  });
});

describe("small formatters", () => {
  it("formatPp signs the difference and always shows points", () => {
    expect(formatPp(2.5)).toBe("+2.5 pp");
    expect(formatPp(-2.5)).toBe("-2.5 pp");
    expect(formatPp(0)).toBe("0.0 pp");
  });

  // TZ-independent on purpose: a zone-less identity timestamp must name the
  // same instant as its explicit-UTC spelling in EVERY runner timezone.
  it("formatUtcClock shows the instant's own UTC clock, not the reader's", () => {
    // The usage page buckets by UTC day, so a timestamp beside those buckets
    // has to read in the same zone or one event carries two dates.
    expect(formatUtcClock("2026-08-16 16:46:32.000", "d MMM HH:mm")).toBe("16 Aug 16:46");
    expect(formatUtcClock("2026-08-16T16:46:32Z", "d MMM HH:mm")).toBe("16 Aug 16:46");
  });

  it("formatUtcInstant reads a zone-less timestamp as UTC, not local", () => {
    expect(formatUtcInstant("2026-08-01T10:15:00.000000", "d MMM yyyy, HH:mm")).toBe(
      formatUtcInstant("2026-08-01T10:15:00Z", "d MMM yyyy, HH:mm"),
    );
    expect(
      formatUtcInstant("2026-08-01T10:15:00+02:00", "HH:mm"),
    ).toBe(formatUtcInstant("2026-08-01T08:15:00Z", "HH:mm"));
  });

  // Same trap as the instant: read as local time, a zone-less journal entry
  // would age by the viewer's offset — hours out on a fresh decision.
  it("formatUtcAge reads a zone-less timestamp as UTC too", () => {
    const now = new Date("2026-08-08T10:15:00Z");

    expect(formatUtcAge("2026-08-01T10:15:00.000000", now)).toBe("7 days ago");
    expect(formatUtcAge("2026-08-01T10:15:00Z", now)).toBe("7 days ago");
  });
});

