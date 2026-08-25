import { describe, expect, it } from "vitest";

import {
  breakdownHeading,
  dimensionDescription,
  dimensionName,
} from "@/lib/metrics/dimension-labels";

describe("dimension labels", () => {
  it("reads an underscored key as words", () => {
    expect(dimensionName("branch_scope")).toBe("Branch scope");
    expect(dimensionName("destination_branch")).toBe("Destination branch");
    expect(dimensionName("source")).toBe("Source");
  });

  it("keeps the mid-sentence form lowercase", () => {
    expect(dimensionDescription("branch_scope")).toBe("branch scope");
    expect(dimensionDescription("category")).toBe("category");
  });

  it("names the question a curated split answers, not the field it reads", () => {
    expect(breakdownHeading(["branch_scope"])).toBe("Default branch vs other");
  });

  it("humanises any other key behind a By, so a new dimension needs no entry", () => {
    expect(breakdownHeading(["source"])).toBe("By source");
    expect(breakdownHeading(["destination_branch"])).toBe(
      "By destination branch"
    );
  });

  it("falls back for a cross-tab, because a curated phrase names one split", () => {
    expect(breakdownHeading(["source", "branch_scope"])).toBe(
      "By source / branch scope"
    );
  });

  // A key inherited from Object.prototype must take the fallback path, not
  // resolve to whatever the prototype holds there — `in` would have returned a
  // function for `constructor` and rendered it as a heading.
  it.each(["constructor", "toString", "__proto__", "hasOwnProperty"])(
    "treats %s as an ordinary key rather than a curated entry",
    (inherited) => {
      expect(breakdownHeading([inherited])).toMatch(/^By /);
    }
  );
});
