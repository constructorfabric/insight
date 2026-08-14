import { describe, expect, it } from "vitest";

import { servingBasepath } from "./base-path";

describe("servingBasepath", () => {
  it("is / for the main deployment", () => {
    expect(servingBasepath("http://stand.test/")).toBe("/");
  });

  it("is the experiment prefix inside a preview", () => {
    expect(servingBasepath("http://stand.test/exp/demo/")).toBe("/exp/demo/");
    expect(servingBasepath("http://stand.test/exp/widget-alpha/")).toBe(
      "/exp/widget-alpha/"
    );
  });

  it("defaults to the document base (jsdom root)", () => {
    expect(servingBasepath()).toBe("/");
  });
});
