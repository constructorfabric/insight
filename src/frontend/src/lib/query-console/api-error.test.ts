import { describe, expect, it } from "vitest";

import { AnalyticsApiError } from "@/api/analytics-client";
import { IdentityApiError } from "@/api/identity-client";

import { apiErrorReason } from "./api-error";

const FALLBACK = "fallback message";

describe("apiErrorReason", () => {
  it("returns the first field-violation description from a canonical body", () => {
    const err = new AnalyticsApiError(400, {
      context: {
        field_violations: [
          { field: "sql", description: "not a single SELECT" },
        ],
      },
    });
    expect(apiErrorReason(err, FALLBACK)).toBe("not a single SELECT");
  });

  it("unwraps an IdentityApiError the same way — both services speak problem+json", () => {
    const err = new IdentityApiError(400, {
      context: {
        field_violations: [{ field: "q", description: "search terms are required" }],
      },
    });
    expect(apiErrorReason(err, FALLBACK)).toBe("search terms are required");
  });

  it("surfaces context.reason when there are no field violations — the 403 shape", () => {
    const err = new IdentityApiError(403, {
      context: { reason: "admin role required for this operation" },
    });
    expect(apiErrorReason(err, FALLBACK)).toBe(
      "admin role required for this operation",
    );
  });

  it("prefers the field violation when a body carries both", () => {
    const err = new IdentityApiError(400, {
      context: {
        reason: "less specific",
        field_violations: [{ field: "q", description: "more specific" }],
      },
    });
    expect(apiErrorReason(err, FALLBACK)).toBe("more specific");
  });

  it("falls back when the error is not an AnalyticsApiError", () => {
    expect(apiErrorReason(new Error("unexpected failure"), FALLBACK)).toBe(
      FALLBACK
    );
    expect(apiErrorReason("nope", FALLBACK)).toBe(FALLBACK);
  });

  it("falls back for malformed or empty bodies", () => {
    for (const body of [
      null,
      undefined,
      {},
      { context: null },
      { context: {} },
      { context: { field_violations: [] } },
      { context: { field_violations: [{ field: "x" }] } },
      { context: { field_violations: [{ description: 42 }] } },
      { context: { reason: 42 } },
      { context: { reason: "  " } },
    ]) {
      expect(apiErrorReason(new AnalyticsApiError(400, body), FALLBACK)).toBe(
        FALLBACK
      );
    }
  });
});
