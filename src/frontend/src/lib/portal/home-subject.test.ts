import { describe, expect, it } from "vitest";

import { homeItemShown, homeSubject } from "./home-subject";

describe("homeSubject", () => {
  it.each([
    ["a viewer with a cohort", true, "team"],
    ["a viewer without a cohort", false, "me"],
  ] as const)("reads %s", (_case, canSeeOthers, expected) => {
    expect(homeSubject(canSeeOthers), `case: ${_case}`).toBe(expected);
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
