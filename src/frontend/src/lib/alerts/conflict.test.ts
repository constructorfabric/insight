import { describe, expect, it } from "vitest";

import { CustomApiError } from "@/api/custom-client";

import { isRevisionConflict } from "./conflict";

describe("isRevisionConflict", () => {
  it.each([
    ["a revision conflict", 409, "revision", true],
    ["the alert limit", 409, "limit", false],
    ["a revision named on a 400", 400, "revision", false],
  ])("tells %s apart", (_case, status, type, expected) => {
    const error = new CustomApiError(status, {
      context: { violations: [{ type, subject: "x", description: "y" }] },
    });

    expect(isRevisionConflict(error)).toBe(expected);
  });
});
