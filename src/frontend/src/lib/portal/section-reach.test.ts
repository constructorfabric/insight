import { describe, expect, it } from "vitest";

import { sectionReach } from "./section-reach";

describe("sectionReach", () => {
  it.each([
    [
      "a section with readings",
      { hasData: true, peersHaveData: true },
      "measured",
    ],
    [
      "a fed section the person did none of",
      { hasData: false, peersHaveData: true },
      "quiet",
    ],
    [
      "a section nothing feeds",
      { hasData: false, peersHaveData: false },
      "unconnected",
    ],
  ] as const)("reads %s", (_case, standing, expected) => {
    expect(sectionReach(standing), `case: ${_case}`).toBe(expected);
  });
});
