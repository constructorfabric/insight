import { describe, expect, it } from "vitest";

import { getInitials } from "@/lib/insight/get-initials";

describe("getInitials", () => {
  it("returns an empty string for missing names", () => {
    expect(getInitials(null)).toBe("");
    expect(getInitials(undefined)).toBe("");
    expect(getInitials("")).toBe("");
  });

  it("uses the first letter of a single name", () => {
    expect(getInitials("alice")).toBe("A");
  });

  it("uses the first two words only", () => {
    expect(getInitials("Alice Kim")).toBe("AK");
    expect(getInitials("Alice van der Berg")).toBe("AV");
  });

  it("tolerates surrounding and repeated whitespace", () => {
    expect(getInitials("  bob \t park  ")).toBe("BP");
  });
});
