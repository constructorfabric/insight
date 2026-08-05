import { describe, expect, it } from "vitest";

import { formatGapMagnitude } from "@/lib/metrics/gap";

describe("formatGapMagnitude", () => {
  it("renders percent metrics as points, never a relative percent or ratio", () => {
    // Focus 55% vs median 90%: the gap is 35 points below, not "-39%".
    expect(
      formatGapMagnitude({
        value: 55,
        median: 90,
        gapPct: (55 - 90) / 90,
        gapDelta: 55 - 90,
        format: "percent",
        unit: null,
      })
    ).toBe("-35 pp");
  });

  it("computes points from the displayed operands so the arithmetic closes", () => {
    // 54.7% renders as 55% and the median as 90% — the gap must reconcile
    // with those (55 + 35 = 90), not the raw fractions (35.3).
    expect(
      formatGapMagnitude({
        value: 54.7,
        median: 90,
        gapPct: (54.7 - 90) / 90,
        gapDelta: 54.7 - 90,
        format: "percent",
        unit: null,
      })
    ).toBe("-35 pp");
  });

  it("drops the gap when rounding collapses it to zero", () => {
    // 89.6% and 90.4% both render as 90% — a "-0.8 pp" beside two equal
    // numbers reads as a bug.
    expect(
      formatGapMagnitude({
        value: 89.6,
        median: 90.4,
        gapPct: (89.6 - 90.4) / 90.4,
        gapDelta: 89.6 - 90.4,
        format: "percent",
        unit: null,
      })
    ).toBeNull();
    // Same for a sub-half-percent relative gap on a count metric.
    expect(
      formatGapMagnitude({
        value: 1001,
        median: 1000,
        gapPct: 1 / 1000,
        gapDelta: 1,
        format: "integer",
        unit: "lines",
      })
    ).toBeNull();
  });

  it("uses a multiple far above the median", () => {
    expect(
      formatGapMagnitude({
        value: 12424,
        median: 3563,
        gapPct: (12424 - 3563) / 3563,
        gapDelta: 12424 - 3563,
        format: "integer",
        unit: "lines",
      })
    ).toBe("3.5×");
  });

  it("uses a signed percent for sub-2x gaps", () => {
    expect(
      formatGapMagnitude({
        value: 3,
        median: 5,
        gapPct: (3 - 5) / 5,
        gapDelta: 3 - 5,
        format: "integer",
        unit: "tasks",
      })
    ).toBe("-40%");
  });

  it("falls back to a signed absolute delta when the median is ~0", () => {
    expect(
      formatGapMagnitude({
        value: 2,
        median: 0,
        gapPct: null,
        gapDelta: 2,
        format: "integer",
        unit: "lines",
      })
    ).toBe("+2 lines");
  });
});
