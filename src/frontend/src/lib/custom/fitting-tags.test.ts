import { describe, expect, it } from "vitest";

import { fittingCount, type TagRow } from "./fitting-tags";

const ROW: TagRow = {
  widths: [60, 60, 60],
  available: 300,
  gap: 6,
  moreWidth: 30,
  overflowing: false,
};

describe("fittingCount", () => {
  it.each<[string, Partial<TagRow>, number]>([
    ["every chip when the row holds them all", {}, 3],
    ["every chip when they fill the row exactly", { available: 192 }, 3],
    ["only whole chips beside +N once one would be cut", { available: 191 }, 2],
    [
      "room for +N even when the shown chips alone would fit",
      { available: 192, overflowing: true },
      2,
    ],
    ["one chip, truncated, when none fits whole", { available: 40 }, 1],
    ["nothing when there are no tags", { widths: [] }, 0],
    [
      "the measured row before layout, where every width is zero",
      { widths: [0, 0, 0], available: 0, gap: 0, moreWidth: 0 },
      3,
    ],
  ])("keeps %s", (_case, row, expected) => {
    expect(fittingCount({ ...ROW, ...row }), `case: ${_case}`).toBe(expected);
  });
});
