import { describe, expect, it } from "vitest";

import { AnalyticsApiError } from "@/api/analytics-client";

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

  it("falls back when the error is not an AnalyticsApiError", () => {
    expect(apiErrorReason(new Error("boom"), FALLBACK)).toBe(FALLBACK);
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
    ]) {
      expect(apiErrorReason(new AnalyticsApiError(400, body), FALLBACK)).toBe(
        FALLBACK
      );
    }
  });
});
