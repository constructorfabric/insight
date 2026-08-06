import { describe, expect, it } from "vitest";

import { formatAxisTick } from "./format";

/**
 * Tick labels have two jobs that pull against each other: fit the ~28px axis
 * gutter, and not lie about where the gridline sits.
 */
describe("formatAxisTick", () => {
  it("keeps four digits literal — they fit, and they are exact", () => {
    // A tick at 2250 labelled "2.3k" would misplace the gridline it sits on.
    expect(formatAxisTick(2250)).toBe("2250");
    expect(formatAxisTick(9000)).toBe("9000");
    expect(formatAxisTick(750)).toBe("750");
    expect(formatAxisTick(0)).toBe("0");
  });

  it("abbreviates from five digits, where the literal no longer fits", () => {
    expect(formatAxisTick(10_000)).toBe("10k");
    expect(formatAxisTick(36_000)).toBe("36k");
    expect(formatAxisTick(102_806)).toBe("103k");
  });

  it("switches to millions before k labels grow long again", () => {
    expect(formatAxisTick(1_000_000)).toBe("1M");
    expect(formatAxisTick(2_400_000)).toBe("2.4M");
  });

  it("survives negatives (deltas, net change axes)", () => {
    expect(formatAxisTick(-2250)).toBe("-2250");
    expect(formatAxisTick(-36_000)).toBe("-36k");
  });

  it("does not print a trailing .0", () => {
    expect(formatAxisTick(1500)).toBe("1500");
    expect(formatAxisTick(3)).toBe("3");
    expect(formatAxisTick(2.5)).toBe("2.5");
  });

  it("stays inside the gutter: no label longer than four characters", () => {
    for (const v of [0, 7.5, 750, 2250, 9000, 10_000, 36_000, 999_000, 2_400_000]) {
      expect(formatAxisTick(v).length, `${v}`).toBeLessThanOrEqual(4);
    }
  });
});
