import { describe, expect, it } from "vitest";

import type { DrilldownColumn } from "@/api/custom-client";

import { drawable } from "./drilldown";

const column = (
  key: string,
  type: DrilldownColumn["type"],
  percent = false
): DrilldownColumn => ({ key, label: key, type, sortable: true, percent });

describe("drawable", () => {
  it.each([
    ["an identifier", column("issue", "number"), 4385, 4385],
    ["a large count", column("lines", "number"), 1234567, 1234567],
    ["a fraction", column("share", "number"), 0.3333, 0.3333],
    ["a percent, marked as one", column("rate", "number", true), 42.5, "42.5%"],
    ["a missing number", column("lines", "number"), null, null],
    ["a list", column("assignees", "string"), ["a", "b"], ["a", "b"]],
  ])("keeps %s as the record holds it", (_case, col, value, shown) => {
    expect(
      drawable({ values: { [col.key]: value } }, [col]).values[col.key],
      _case
    ).toEqual(shown);
  });

  it("links a cell that is an absolute URL", () => {
    const url = "https://example.com/pull/1";
    expect(
      drawable({ values: { pr: url } }, [column("pr", "string")]).links
    ).toEqual({
      pr: url,
    });
  });
});
