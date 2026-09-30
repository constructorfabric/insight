import { afterEach, describe, expect, it, vi } from "vitest";

import { sameTagName } from "./tag-names";

describe("sameTagName", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetModules();
  });

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

  it("matches as the service does whatever the reader's locale", async () => {
    const RealCollator = Intl.Collator;
    const turkish = Object.create(Intl) as typeof Intl;
    turkish.Collator = function (
      locales?: string | string[],
      options?: Intl.CollatorOptions
    ) {
      return new RealCollator(locales ?? "tr", options);
    } as unknown as typeof Intl.Collator;
    vi.stubGlobal("Intl", turkish);
    vi.resetModules();

    const { sameTagName: underTurkish } = await import("./tag-names");

    expect(underTurkish("hiring", "HIRING")).toBe(true);
  });
});
