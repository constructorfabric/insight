import { describe, expect, it } from "vitest";

import {
  computeDelta,
  deltaStatus,
  formatTileDelta,
} from "@/lib/metrics/delta";

describe("computeDelta", () => {
  it("computes relative change for sums", () => {
    expect(computeDelta(120, 100, "sum", "integer")).toEqual({
      kind: "percent_change",
      value: 20,
    });
    expect(computeDelta(80, 100, "sum", "integer")).toEqual({
      kind: "percent_change",
      value: -20,
    });
  });

  it("computes percentage-point change for percent-formatted ratios", () => {
    // 77% acceptance vs 72% last period is +5 pp, not +6.9%.
    const delta = computeDelta(77.04, 72.04, "ratio", "percent");
    expect(delta?.kind).toBe("pp_change");
    expect(delta?.value).toBeCloseTo(5);
  });

  it("treats non-percent ratios as relative change", () => {
    expect(computeDelta(3, 2, "ratio", "decimal")).toEqual({
      kind: "percent_change",
      value: 50,
    });
  });

  it("returns null for missing values and zero baselines", () => {
    expect(computeDelta(null, 100, "sum", "integer")).toBeNull();
    expect(computeDelta(100, null, "sum", "integer")).toBeNull();
    expect(computeDelta(100, 0, "sum", "integer")).toBeNull();
    expect(computeDelta(Number.NaN, 100, "sum", "integer")).toBeNull();
  });

  it("uses the absolute baseline for negative previous values", () => {
    expect(computeDelta(-50, -100, "sum", "integer")).toEqual({
      kind: "percent_change",
      value: 50,
    });
  });
});

describe("formatTileDelta", () => {
  it("keeps ordinary relative changes as a signed percent", () => {
    expect(formatTileDelta({ kind: "percent_change", value: 20 })).toBe("+20%");
    expect(formatTileDelta({ kind: "percent_change", value: -45 })).toBe(
      "-45%"
    );
    expect(formatTileDelta({ kind: "percent_change", value: 99 })).toBe("+99%");
  });

  it("switches a runaway increase to a multiple at the shared threshold", () => {
    // 278 tasks vs 5 last period is +5460% — unreadable as a percent.
    expect(formatTileDelta({ kind: "percent_change", value: 5460 })).toBe(
      "56×"
    );
    expect(formatTileDelta({ kind: "percent_change", value: 100 })).toBe("2×");
    expect(formatTileDelta({ kind: "percent_change", value: 150 })).toBe(
      "2.5×"
    );
  });

  it("never switches the bounded downside or pp changes to a multiple", () => {
    expect(formatTileDelta({ kind: "percent_change", value: -100 })).toBe(
      "-100%"
    );
    expect(formatTileDelta({ kind: "pp_change", value: 250 })).toBe("+250 pp");
  });

  it("suppresses deltas that round to zero", () => {
    expect(formatTileDelta({ kind: "percent_change", value: 0.4 })).toBeNull();
    expect(formatTileDelta({ kind: "pp_change", value: -0.4 })).toBeNull();
  });
});

describe("deltaStatus deadband", () => {
  it("withholds the colour from a move too small to mean anything", () => {
    // The number is still shown; only the alarm is withheld. A badge that
    // paints -1% in the same red as -70% teaches the reader to ignore red.
    expect(
      deltaStatus({ kind: "percent_change", value: -1 }, "higher_is_better")
    ).toBe("neutral");
    expect(
      deltaStatus({ kind: "pp_change", value: 1 }, "higher_is_better")
    ).toBe("neutral");
  });

  it("still judges a move that clears the deadband", () => {
    expect(
      deltaStatus({ kind: "percent_change", value: -13 }, "higher_is_better")
    ).toBe("bad");
    expect(
      deltaStatus({ kind: "pp_change", value: 5 }, "higher_is_better")
    ).toBe("good");
  });
});
