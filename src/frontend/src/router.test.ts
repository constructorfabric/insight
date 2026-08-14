import { describe, expect, it } from "vitest";

import { router } from "./router";

describe("router", () => {
  it("basepath follows the document base", () => {
    // jsdom serves at "/": the runtime <base> contract maps that to basepath "/".
    expect(router.basepath).toBe("/");
  });
});
