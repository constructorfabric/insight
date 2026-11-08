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
    ["any theme, for a team", "team", "attention", true],
    ["an item with no own view, for one person", "me", "nope", false],
    ["By direction, for one person", "me", "by-direction", true],
    ["the glance, for one person", "me", "at-a-glance", true],
  ] as const)("decides %s", (_case, subject, item, expected) => {
    expect(homeItemShown(subject, item), `case: ${_case}`).toBe(expected);
  });
});
