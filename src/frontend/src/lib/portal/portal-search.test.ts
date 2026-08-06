import { describe, expect, it } from "vitest";

import { validatePortalSearch } from "./portal-search";

/**
 * Search params are user-editable text and arrive from links other people
 * wrote. The contract is to DEGRADE — drop what makes no sense and fall back to
 * a computed default — never to error, because the alternative is an error
 * boundary in place of a dashboard over one mistyped character.
 */
describe("validatePortalSearch", () => {
  it("keeps the navigation keys it recognises", () => {
    expect(
      validatePortalSearch({ zone: "directions", dir: "dev", lens: "Delivery" }),
    ).toMatchObject({ zone: "directions", dir: "dev", lens: "Delivery" });
  });

  it("lowercases the scope — an email is case-insensitive but our keys are not", () => {
    expect(validatePortalSearch({ scope: "Head@X.COM" }).scope).toBe("head@x.com");
  });

  it("drops empty and non-string values rather than passing them on", () => {
    const out = validatePortalSearch({ zone: "", item: 42, dir: null });
    expect(out.zone).toBeUndefined();
    expect(out.item).toBeUndefined();
    expect(out.dir).toBeUndefined();
  });

  it("omits `direct` unless it is truthy — no defaults in a shared URL", () => {
    expect(validatePortalSearch({ direct: "false" })).not.toHaveProperty("direct");
    expect(validatePortalSearch({})).not.toHaveProperty("direct");
    expect(validatePortalSearch({ direct: "true" }).direct).toBe(true);
  });

  it("ignores an unknown period preset", () => {
    expect(validatePortalSearch({ period: "fortnight" }).period).toBeUndefined();
    expect(validatePortalSearch({ period: "quarter" }).period).toBe("quarter");
  });

  describe("custom range", () => {
    it("keeps a well-formed, correctly ordered pair", () => {
      const out = validatePortalSearch({ from: "2026-01-01", to: "2026-01-31" });
      expect(out).toMatchObject({ from: "2026-01-01", to: "2026-01-31" });
    });

    it("drops an INVERTED pair instead of letting it reach the range assert", () => {
      // Well-formed and nonsensical: this used to pass validation and throw
      // downstream, turning a bad link into an error boundary.
      expect(
        validatePortalSearch({ from: "2026-07-30", to: "2026-01-01" }),
      ).not.toHaveProperty("from");
    });

    it("drops a pair that exceeds the maximum span", () => {
      expect(
        validatePortalSearch({ from: "2000-01-01", to: "2026-01-01" }),
      ).not.toHaveProperty("from");
    });

    it("drops a half-specified range — one bound cannot resolve a window", () => {
      expect(validatePortalSearch({ from: "2026-01-01" })).not.toHaveProperty("from");
      expect(validatePortalSearch({ to: "2026-01-31" })).not.toHaveProperty("to");
    });

    it("drops malformed dates", () => {
      expect(
        validatePortalSearch({ from: "01/01/2026", to: "31/01/2026" }),
      ).not.toHaveProperty("from");
      expect(
        validatePortalSearch({ from: "2026-13-45", to: "2026-13-46" }),
      ).not.toHaveProperty("from");
    });
  });
});
