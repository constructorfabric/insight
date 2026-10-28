import { describe, expect, it } from "vitest";

import { CustomApiError } from "@/api/custom-client";

import { refusals } from "./violations";

function refused(context: unknown, detail?: string): CustomApiError {
  return new CustomApiError(400, { detail, context });
}

function invalid(violations: unknown[], detail?: string): CustomApiError {
  return refused({ field_violations: violations }, detail);
}

const FALLBACK = "Couldn't store it.";

describe("refusals", () => {
  it("names each violation by the path it names, in the order sent", () => {
    const said = refusals(
      invalid([
        { field: "fields[0].type", description: "unknown type" },
        { field: "title", description: "must not be empty" },
        { field: "title", description: "too long" },
      ]),
      FALLBACK
    );

    expect(said).toEqual([
      "fields[0].type: unknown type",
      "title: must not be empty",
      "title: too long",
    ]);
  });

  it("says a violation that names nothing as it is", () => {
    const said = refusals(
      invalid([{ description: "a dataset needs at least one field" }]),
      FALLBACK
    );

    expect(said).toEqual(["a dataset needs at least one field"]);
  });

  it("leaves out a violation with nothing to say", () => {
    const said = refusals(
      invalid([{ field: "title" }, { field: "limit", description: "wrong" }]),
      FALLBACK
    );

    expect(said).toEqual(["limit: wrong"]);
  });

  // A 409 names things outside the document - the metrics a change would
  // break - by their subject.
  it("says what a precondition names, by its subject", () => {
    const said = refusals(
      refused({
        violations: [
          {
            type: "would_break",
            subject: "lines_per_day",
            description: "reads `day`",
          },
        ],
      }),
      FALLBACK
    );

    expect(said).toEqual(["lines_per_day: reads `day`"]);
  });

  it("falls back to the plain refusal when nothing was named", () => {
    const said = refusals(
      refused(undefined, "the dataset is being removed"),
      FALLBACK
    );

    expect(said).toEqual(["the dataset is being removed"]);
  });

  it("leaves the plain refusal out once a violation was named", () => {
    const said = refusals(
      invalid([{ field: "title", description: "required" }], "invalid body"),
      FALLBACK
    );

    expect(said).toEqual(["title: required"]);
  });

  // A save that failed and shows nothing reads as a save that worked.
  it.each([
    ["a network failure", new TypeError("Failed to fetch")],
    ["a body that is not the service's", new CustomApiError(502, null)],
    [
      "a body with nothing readable",
      new CustomApiError(500, { message: "boom" }),
    ],
  ])("says something for %s", (_, error) => {
    expect(refusals(error, FALLBACK), `should fall back: ${_}`).toEqual([
      FALLBACK,
    ]);
  });

  it("says nothing when there is no error", () => {
    expect(refusals(null, FALLBACK)).toEqual([]);
  });
});
