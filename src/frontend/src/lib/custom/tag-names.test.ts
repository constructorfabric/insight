import { describe, expect, it } from "vitest";

import { sameTagName } from "./tag-names";

describe("sameTagName", () => {
  it.each([
    ["Ops", "ops"],
    ["Ops", "OPS"],
    ["Qualität", "QUALITÄT"],
  ])("treats %s and %s as one tag", (a, b) => {
    expect(sameTagName(a, b)).toBe(true);
  });

  it.each([
    ["Resume", "Résumé"],
    ["Ops", "Opsx"],
  ])("keeps %s and %s apart", (a, b) => {
    expect(sameTagName(a, b)).toBe(false);
  });
});
