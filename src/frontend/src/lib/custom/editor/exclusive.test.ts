import { describe, expect, it } from "vitest";

import { alone, called } from "./exclusive";

describe("alone", () => {
  it("moves the mark to the entry it was set on", () => {
    const list = [
      { name: "a", default_clock: true },
      { name: "b" },
      { name: "c", default_clock: true },
    ];

    expect(alone(list, 1, "default_clock")).toEqual([
      { name: "a" },
      { name: "b", default_clock: true },
      { name: "c" },
    ]);
  });

  it("marks an entry that is not an object without losing the others", () => {
    expect(alone([1, { name: "b" }], 0, "flag")).toEqual([
      { flag: true },
      { name: "b" },
    ]);
  });

  it("marks nothing in what is not a list", () => {
    expect(alone("nope", 0, "flag")).toEqual([]);
  });
});

describe("called", () => {
  it("reads what each entry is called, skipping the unnamed", () => {
    const list = [{ name: "day" }, { path: "x" }, { name: "" }, { name: "n" }];

    expect(called(list, "name")).toEqual(["day", "n"]);
  });

  it("reads nothing from what is not a list", () => {
    expect(called(undefined, "name")).toEqual([]);
  });
});
