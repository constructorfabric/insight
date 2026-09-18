import { describe, expect, it } from "vitest";

import {
  RANGE_PRESETS,
  isRangeToken,
  rangeLabel,
  toInterval,
} from "./time-range";

describe("isRangeToken", () => {
  it("accepts every preset the server answers", () => {
    for (const preset of RANGE_PRESETS) {
      expect(isRangeToken(preset.token), preset.token).toBe(true);
    }
  });

  it("accepts an ISO interval whose end is after its start", () => {
    expect(isRangeToken("2026-08-01/2026-09-01")).toBe(true);
  });

  it.each([
    ["a duration nobody offers", "P14D"],
    ["one date alone", "2026-09-01"],
    ["an empty span", "2026-09-01/2026-09-01"],
    ["a reversed span", "2026-09-02/2026-09-01"],
    ["a day that does not exist", "2026-02-30/2026-03-01"],
    ["an unpadded date", "2026-9-1/2026-09-02"],
    ["a third date", "2026-09-01/2026-09-02/2026-09-03"],
    ["nothing at all", ""],
  ])("refuses %s", (_why, token) => {
    expect(isRangeToken(token)).toBe(false);
  });
});

describe("rangeLabel", () => {
  it("reads a preset as a person would say it", () => {
    expect(rangeLabel("P30D")).toBe("Last 30 days");
    expect(rangeLabel("inf")).toBe("All time");
    // P1Y is 365 rolling days, not a calendar year — the label says which.
    expect(rangeLabel("P1Y")).toBe("Last 365 days");
  });

  it("reads an interval as its two dates", () => {
    expect(rangeLabel("2026-08-01/2026-09-01")).toBe("2026-08-01 – 2026-09-01");
  });
});

describe("toInterval", () => {
  it("splits a valid interval into the dates it names", () => {
    expect(toInterval("2026-08-01/2026-09-01")).toEqual({
      from: "2026-08-01",
      to: "2026-09-01",
    });
  });

  it("has nothing to split out of a preset", () => {
    expect(toInterval("P30D")).toBeUndefined();
  });
});
