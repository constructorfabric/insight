import { describe, expect, it } from "vitest";

import type { DrilldownColumn } from "@/api/custom-client";

import { drawable } from "./drilldown";

const column = (
  key: string,
  type: DrilldownColumn["type"],
  { percent = false, exact = false } = {}
): DrilldownColumn => ({
  key,
  label: key,
  type,
  sortable: true,
  percent,
  exact,
});

describe("drawable", () => {
  it.each([
    ["an identifier", column("issue", "number"), 4385, 4385],
    ["a large count", column("lines", "number"), 1234567, 1234567],
    [
      "a 64-bit count sent as text",
      column("bytes", "number"),
      "18446744073709551615",
      "18446744073709551615",
    ],
    ["a missing number", column("lines", "number"), null, null],
    ["a list", column("assignees", "string"), ["a", "b"], ["a", "b"]],
    [
      "a fraction the metric marks exact",
      column("share", "number", { exact: true }),
      0.3333,
      0.3333,
    ],
    [
      "a percent the metric marks exact, marked as one",
      column("rate", "number", { percent: true, exact: true }),
      33.333333333333336,
      "33.333333333333336%",
    ],
  ])("keeps %s as the record holds it", (_case, col, value, shown) => {
    expect(
      drawable({ values: { [col.key]: value } }, [col]).values[col.key],
      _case
    ).toEqual(shown);
  });

  it.each([
    ["a fraction", column("share", "number"), 0.3333, "0.33"],
    ["a float sum", column("cost", "number"), 1234.5678901234999, "1234.57"],
    ["a sum that should be zero", column("cost", "number"), -2.1e-16, "0"],
    ["a short fraction", column("cost", "number"), 2.5, "2.5"],
    [
      "a percent",
      column("rate", "number", { percent: true }),
      11.111111111111112,
      "11.11%",
    ],
    ["a whole percent", column("rate", "number", { percent: true }), 50, "50%"],
  ])(
    "writes %s to at most two decimals unless exact",
    (_case, col, value, shown) => {
      expect(
        drawable({ values: { [col.key]: value } }, [col]).values[col.key],
        _case
      ).toEqual(shown);
    }
  );

  it("links a cell that is an absolute URL", () => {
    const url = "https://example.com/pull/1";
    expect(
      drawable({ values: { pr: url } }, [column("pr", "string")]).links
    ).toEqual({
      pr: url,
    });
  });
});
