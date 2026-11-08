import { describe, expect, it } from "vitest";

import { formatSources, normalizeSources, parseSources } from "./sources";

describe("parseSources", () => {
  it("reads nothing as no sources", () => {
    expect(parseSources(undefined)).toEqual([]);
    expect(parseSources("")).toEqual([]);
  });

  it("splits, trims, drops blanks, dedupes and sorts", () => {
    expect(parseSources(" gitlab, github,,github ,")).toEqual([
      "github",
      "gitlab",
    ]);
  });
});

describe("formatSources", () => {
  it("round-trips a sorted, deduped list", () => {
    expect(formatSources(["gitlab", "github", "gitlab"])).toBe("github,gitlab");
    expect(parseSources(formatSources(["gitlab", "github"]))).toEqual([
      "github",
      "gitlab",
    ]);
  });

  it("formats an empty list as absent", () => {
    expect(formatSources([])).toBeUndefined();
  });
});

describe("normalizeSources", () => {
  it("trims, drops blanks, dedupes and sorts", () => {
    expect(normalizeSources([" gitlab", "github", "", "gitlab "])).toEqual([
      "github",
      "gitlab",
    ]);
  });
});
