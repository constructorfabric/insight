import { describe, expect, it } from "vitest";

import { nextInWalk } from "./walk";

describe("nextInWalk", () => {
  it("returns the key after the current one", () => {
    expect(nextInWalk(["a", "b", "c"], "a")).toBe("b");
    expect(nextInWalk(["a", "b", "c"], "b")).toBe("c");
  });

  it("returns null after the last key", () => {
    expect(nextInWalk(["a", "b", "c"], "c")).toBeNull();
  });

  it("returns null for a key outside the walk", () => {
    expect(nextInWalk(["a", "b"], "z")).toBeNull();
  });

  it("returns null for an empty walk", () => {
    expect(nextInWalk([], "a")).toBeNull();
  });
});
