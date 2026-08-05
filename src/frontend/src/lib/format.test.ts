import { describe, expect, it } from "vitest";

import {
  NO_METRIC_VALUE,
  formatMetricNumber,
  formatMetricValue,
  formatPp,
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

});

