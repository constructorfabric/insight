import { describe, expect, it } from "vitest";

import { toBarRows, type BarEntry } from "./bar-rows";

const entry = (
  label: string,
  value: number,
  split?: [string, string, number][]
): BarEntry => ({
  label,
  value,
  split: split
    ? new Map(split.map(([seed, l, v]) => [seed, { seed, label: l, value: v }]))
    : undefined,
});

describe("toBarRows", () => {
  it("keys a row by the dimension value, not by what it shows", () => {
    const rows = toBarRows(
      new Map([
        ["src-a:acme/api", entry("acme/api", 10)],
        ["src-b:acme/api", entry("acme/api", 4)],
      ])
    );

    expect(rows.map((r) => r.key)).toEqual([
      "src-a:acme/api",
      "src-b:acme/api",
    ]);
    expect(new Set(rows.map((r) => r.label)).size).toBe(1);
  });

  it("keeps the trunk reading first however small it is", () => {
    const rows = toBarRows(
      new Map([
        [
          "acme/api",
          entry("acme/api", 100, [
            ["non_default", "Other branches", 90],
            ["default", "Default branch", 10],
          ]),
        ],
      ]),
      "branch_scope"
    );

    expect(rows[0]?.segments?.map((s) => s.seed)).toEqual([
      "default",
      "non_default",
    ]);
  });

  it("sorts by size where the split declares no order", () => {
    const rows = toBarRows(
      new Map([
        [
          "acme/api",
          entry("acme/api", 100, [
            ["small", "Small", 10],
            ["big", "Big", 90],
          ]),
        ],
      ]),
      "category"
    );

    expect(rows[0]?.segments?.map((s) => s.seed)).toEqual(["big", "small"]);
  });

  it("puts a segment the declared order does not name after the ones it does", () => {
    const rows = toBarRows(
      new Map([
        [
          "acme/api",
          entry("acme/api", 100, [
            ["unsplit", "Unknown", 50],
            ["default", "Default branch", 30],
            ["non_default", "Other branches", 20],
          ]),
        ],
      ]),
      "branch_scope"
    );

    expect(rows[0]?.segments?.map((s) => s.seed)).toEqual([
      "default",
      "non_default",
      "unsplit",
    ]);
  });
});
