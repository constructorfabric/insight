import { describe, expect, it } from "vitest";

import {
  categoryTick,
  compactNumber,
  groupedNumber,
  shortDate,
  spansYears,
  unitFor,
} from "./chart-format";

describe("chart-format", () => {
  it("writes a figure with separators, because someone reads it", () => {
    expect(groupedNumber(163997048)).toBe("163,997,048");
    expect(groupedNumber("24887")).toBe("24,887");
  });

  it("writes an axis tick compactly, because the shape is the point", () => {
    expect(compactNumber(24887)).toBe("24.9K");
    expect(compactNumber(400000)).toBe("400K");
  });

  it("signs a rate as one, and rounds off the float's tail", () => {
    expect(groupedNumber(83.91167192429022, "%")).toBe("83.9%");
    expect(compactNumber(83.91167192429022, "%")).toBe("83.9%");
    // Nothing to sign is nothing added — an em dash stays an em dash.
    expect(groupedNumber(null, "%")).toBe("—");
  });

  it("takes the unit from the metric, which is the only thing that knows", () => {
    expect(unitFor(["pass_rate"], "pass_rate")).toBe("%");
    expect(unitFor(["pass_rate"], "runs")).toBe("");
    expect(unitFor(undefined, "pass_rate")).toBe("");
  });

  it("leaves a missing number as a dash rather than printing null", () => {
    expect(groupedNumber(null)).toBe("—");
    expect(groupedNumber(undefined)).toBe("—");
  });

  it("keeps text that is not a number", () => {
    expect(groupedNumber("insight/insight")).toBe("insight/insight");
  });

  it.each([
    ["2024-11-23", "Nov 23"],
    ["2024-11-23 00:00:00", "Nov 23"],
    ["2024-11-23 00:00:00.000", "Nov 23"],
  ])("reads %s as a date, whatever time the column carries", (value, expected) => {
    expect(shortDate(value)).toBe(expected);
  });

  it("adds the year when the data spans more than one", () => {
    expect(shortDate("2024-11-23", true)).toBe("Nov 23, 2024");
    expect(spansYears(["2024-11-23", "2025-01-04"])).toBe(true);
    expect(spansYears(["2025-11-23", "2025-01-04"])).toBe(false);
  });

  it("cuts a category that would crowd the axis", () => {
    // A commit sha is 40 characters; a whole axis of them is a smear.
    expect(categoryTick("51e8cfdcfa77d673719845850706f2ab")).toBe("51e8cfdcfa77d…");
    expect(categoryTick("insight/insight")).toBe("insight/insig…");
    expect(categoryTick("chatgpt")).toBe("chatgpt");
  });

  it("prefers the date reading over the cut", () => {
    expect(categoryTick("2024-11-23 00:00:00.000")).toBe("Nov 23");
  });
});
