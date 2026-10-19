import { describe, expect, it } from "vitest";

import { CustomApiError } from "@/api/custom-client";

import { savingRefusal } from "./field-refusals";

describe("savingRefusal", () => {
  it("puts each named field's reason on that field", () => {
    const error = new CustomApiError(400, {
      detail: "Request validation failed",
      context: {
        field_violations: [
          {
            field: "interval_secs",
            description: "too short",
            reason: "INVALID",
          },
          { field: "range", description: "too wide", reason: "INVALID" },
          {
            field: "condition.where.all[1].op",
            description: "`contains` does not apply to a number column",
            reason: "INVALID",
          },
          {
            field: "expected_revision",
            description: "stale",
            reason: "INVALID",
          },
        ],
      },
    });

    expect(savingRefusal(error)).toEqual({
      fields: {
        interval_secs: "too short",
        range: "too wide",
        "condition.where.all[1].op":
          "`contains` does not apply to a number column",
      },
    });
  });

  it.each([
    [
      "a refusal about no field of the form",
      new CustomApiError(409, {
        detail: "Failed precondition",
        context: {
          violations: [
            {
              type: "limit",
              subject: "name",
              description: "at most 200 alerts",
            },
          ],
        },
      }),
      "at most 200 alerts",
    ],
    [
      "something that is not a refusal",
      new Error("offline"),
      "Couldn't save the alert.",
    ],
  ])("shows %s above the form", (_case, error, general) => {
    expect(savingRefusal(error)).toEqual({ fields: {}, general });
  });
});
