import { describe, expect, it } from "vitest";

import { CustomApiError } from "@/api/custom-client";

import type { Field } from "./describe";
import { DESCRIPTIONS } from "./kinds";
import { offers, place } from "./violations";

function refused(context: unknown, detail?: string): CustomApiError {
  return new CustomApiError(400, { detail, context });
}

function invalid(violations: unknown[], detail?: string): CustomApiError {
  return refused({ field_violations: violations }, detail);
}

const FORM: readonly Field[] = [
  { name: "title", label: "Title", shape: { of: "text" } },
  {
    name: "fields",
    label: "Fields",
    shape: {
      of: "list",
      entryLabel: "field",
      entry: {
        of: "record",
        fields: [{ name: "type", label: "Type", shape: { of: "text" } }],
      },
    },
  },
];

const FALLBACK = "Couldn't store it.";

describe("offers", () => {
  it("names a row for every field, and for every entry the list has", () => {
    const document = { fields: [{ type: "int" }, { type: "string" }] };

    expect(offers(FORM, document)).toEqual(
      new Set([
        "title",
        "fields",
        "fields[0]",
        "fields[0].type",
        "fields[1]",
        "fields[1].type",
      ])
    );
  });

  it("names no entry for a list the document does not hold", () => {
    expect(offers(FORM, {})).toEqual(new Set(["title", "fields"]));
  });

  // The form draws the chosen variant's rows and no other's.
  it("names only the chosen variant's fields", () => {
    const offered = offers(DESCRIPTIONS.widgets.fields, {
      type: "stat",
      columns: ["x"],
    });

    expect(offered).toContain("type");
    expect(offered).toContain("value");
    expect(offered).not.toContain("columns");
  });

  it("names a dashboard item's row by the kind it carries", () => {
    const offered = offers(DESCRIPTIONS.dashboards.fields, {
      items: [{ heading: "Flow" }],
    });

    expect(offered).toContain("items[0]");
    expect(offered).toContain("items[0].heading");
    expect(offered).not.toContain("items[0].widget");
  });
});

describe("place", () => {
  const document = { title: "T", fields: [{ type: "moment" }] };

  it("puts a violation on the row it names", () => {
    const placed = place(
      invalid([{ field: "fields[0].type", description: "unknown type" }]),
      FORM,
      document,
      FALLBACK
    );

    expect(placed.at.get("fields[0].type")).toBe("unknown type");
    expect(placed.loose).toEqual([]);
  });

  it("puts a violation about an entry on the entry", () => {
    const placed = place(
      invalid([{ field: "fields[0]", description: "a field is an object" }]),
      FORM,
      { fields: [1] },
      FALLBACK
    );

    expect(placed.at.get("fields[0]")).toBe("a field is an object");
  });

  // The service checks more than the form draws; a violation with no row is
  // still said rather than leaving a form that refuses in silence.
  it.each([
    ["a path the form has no field for", "row_identity[3]", document],
    ["an entry past the list's end", "fields[4].type", document],
    ["a field of a variant not chosen", "columns", { type: "stat" }],
  ])("keeps %s, named", (_, field, held) => {
    const fields = field === "columns" ? DESCRIPTIONS.widgets.fields : FORM;
    const placed = place(
      invalid([{ field, description: "wrong" }]),
      fields,
      held,
      FALLBACK
    );

    expect(placed.at.size, `should not place: ${field}`).toBe(0);
    expect(placed.loose).toEqual([`${field}: wrong`]);
  });

  it("keeps a violation that names nothing", () => {
    const placed = place(
      invalid([{ description: "a dataset needs at least one field" }]),
      FORM,
      document,
      FALLBACK
    );

    expect(placed.loose).toEqual(["a dataset needs at least one field"]);
  });

  // One row, two complaints: the first sits on it, the second is still said.
  it("says the second complaint about one row beside the form", () => {
    const placed = place(
      invalid([
        { field: "title", description: "must not be empty" },
        { field: "title", description: "too long" },
      ]),
      FORM,
      document,
      FALLBACK
    );

    expect(placed.at.get("title")).toBe("must not be empty");
    expect(placed.loose).toEqual(["title: too long"]);
  });

  // A 409 names things outside the document - the metrics a change would
  // break - which have no row and are said by name.
  it("says what a precondition names, by its subject", () => {
    const placed = place(
      refused({
        violations: [
          {
            type: "would_break",
            subject: "lines_per_day",
            description: "reads `day`",
          },
        ],
      }),
      FORM,
      document,
      FALLBACK
    );

    expect(placed.at.size).toBe(0);
    expect(placed.loose).toEqual(["lines_per_day: reads `day`"]);
  });

  it("falls back to the plain refusal when nothing was named", () => {
    const placed = place(
      refused(undefined, "the dataset is being removed"),
      FORM,
      document,
      FALLBACK
    );

    expect(placed.loose).toEqual(["the dataset is being removed"]);
  });

  it("leaves the plain refusal out once a violation has been placed", () => {
    const placed = place(
      invalid([{ field: "title", description: "required" }], "invalid body"),
      FORM,
      document,
      FALLBACK
    );

    expect(placed.loose).toEqual([]);
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
    const placed = place(error, FORM, document, FALLBACK);

    expect(placed.loose).toEqual([FALLBACK]);
  });

  it("says nothing when there is no error", () => {
    expect(place(null, FORM, document, FALLBACK).loose).toEqual([]);
  });
});
