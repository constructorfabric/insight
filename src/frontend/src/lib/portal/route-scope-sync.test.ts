import { beforeEach, describe, expect, it } from "vitest";

import { claimScopeSync, resetRouteScopeSync } from "./route-scope-sync";

const A = "aaaaaaaa-1111-4111-8111-111111111111";
const B = "bbbbbbbb-1111-4111-8111-111111111111";

beforeEach(() => resetRouteScopeSync());

describe("claimScopeSync", () => {
  it("claims a person once, so a re-render or remount does not re-sync", () => {
    expect(claimScopeSync(A)).toBe(true);
    expect(claimScopeSync(A)).toBe(false);
    expect(claimScopeSync(A)).toBe(false);
  });

  it("claims again after the person changes — A then B then A", () => {
    // Deliberate, not an oversight: arriving at a person's team IS a
    // navigation. A reader who visits A, picks a different scope, looks at B
    // and then clicks back to A's team must get A's roster. Remembering every
    // person ever seen would leave the scope on B while the route said A.
    expect(claimScopeSync(A)).toBe(true);
    expect(claimScopeSync(B)).toBe(true);
    expect(claimScopeSync(A)).toBe(true);
    // …and still only once per arrival.
    expect(claimScopeSync(A)).toBe(false);
  });

  it("never claims an empty person", () => {
    expect(claimScopeSync("")).toBe(false);
  });

  it("resets, so a test mounting the same person again is not a no-op", () => {
    expect(claimScopeSync(A)).toBe(true);
    resetRouteScopeSync();
    expect(claimScopeSync(A)).toBe(true);
  });
});
