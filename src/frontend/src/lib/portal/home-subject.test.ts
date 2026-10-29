import { describe, expect, it } from "vitest";

import { homeItemShown, homeSubject } from "./home-subject";

describe("homeSubject", () => {
  it.each([
    ["a viewer with a cohort, by default", true, undefined, "team"],
    ["a viewer with a cohort who picked Me", true, "me", "me"],
    ["a viewer without a cohort", false, undefined, "me"],
    ["a viewer without a cohort, whatever the link says", false, "me", "me"],
  ] as const)("reads %s", (_case, canSeeOthers, who, expected) => {
    expect(homeSubject(canSeeOthers, who), `case: ${_case}`).toBe(expected);
  });
});

describe("homeItemShown", () => {
  it.each([
    ["the contribution split, for a team", "team", "contribution", true],
    ["the contribution split, for one person", "me", "contribution", false],
    ["the trend, for one person", "me", "trend", true],
    ["the glance, for one person", "me", "at-a-glance", true],
  ] as const)("decides %s", (_case, subject, item, expected) => {
    expect(homeItemShown(subject, item), `case: ${_case}`).toBe(expected);
  });
});
