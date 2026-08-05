import { describe, expect, it } from "vitest";

import { percentShareLabels } from "@/lib/metrics/shares";

const sum = (labels: string[]) =>
  labels.reduce((total, label) => total + Number(label), 0);

describe("percentShareLabels", () => {
  it("sums to exactly 100 for equal thirds (no 99%)", () => {
    const labels = percentShareLabels([1, 1, 1]);
    expect(sum(labels)).toBe(100);
    expect(labels).toEqual(["34", "33", "33"]);
  });

  it("keeps exact splits unchanged", () => {
    expect(percentShareLabels([80, 20])).toEqual(["80", "20"]);
    expect(percentShareLabels([60, 40])).toEqual(["60", "40"]);
  });

  it("gives a tiny nonzero part a tenth instead of a contradictory 0%", () => {
    const labels = percentShareLabels([1576, 1]);
    expect(labels).toEqual(["99.9", "0.1"]);
    expect(sum(labels)).toBeCloseTo(100);
  });

  it("switches the whole legend to tenths on a sub-1% part", () => {
    const labels = percentShareLabels([995, 5]);
    expect(labels).toEqual(["99.5", "0.5"]);
    expect(sum(labels)).toBeCloseTo(100);
  });

  it("floors ultra-small nonzero parts at 0.1 by taking from the largest", () => {
    const labels = percentShareLabels([99999, 1]);
    expect(labels).toEqual(["99.9", "0.1"]);
    expect(sum(labels)).toBeCloseTo(100);
  });

  it("returns zeros for a non-positive total", () => {
    expect(percentShareLabels([0, 0])).toEqual(["0", "0"]);
    expect(percentShareLabels([])).toEqual([]);
  });

  it("gives a single part the whole 100%", () => {
    expect(percentShareLabels([42])).toEqual(["100"]);
  });
});
