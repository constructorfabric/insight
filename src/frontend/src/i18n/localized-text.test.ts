import { describe, expect, it } from "vitest";

import i18n from "@/i18n";
import { resolveText } from "@/i18n/localized-text";

describe("resolveText", () => {
  it("reads localized text from the active locale", () => {
    expect(resolveText(i18n.t, { key: "worth_a_look.title" })).toBe("Worth a look");
  });

  it("passes literal text through unchanged", () => {
    expect(resolveText(i18n.t, "By direction")).toBe("By direction");
  });
});
