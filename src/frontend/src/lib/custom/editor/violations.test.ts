import { describe, expect, it } from "vitest";

import { CustomApiError } from "@/api/custom-client";

import type { Field } from "./describe";
import { DESCRIPTIONS } from "./kinds";
import { offers, place } from "./violations";

function refused(violations: unknown[], detail?: string): CustomApiError {
  return new CustomApiError(400, { detail, context: { violations } });
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

describe("offers", () => {
  it("names a path for every field, however deep", () => {
    expect(offers(FORM)).toEqual(
      new Set(["title", "fields", "fields[]", "fields[].type"])
    );
  });

  it("offers every variant's fields, since any of them may be chosen", () => {
    const offered = offers(DESCRIPTIONS.widgets.fields);

    // A variant's fields sit beside the choice, which is where they are sent.
    expect(offered).toContain("type");
    expect(offered).toContain("columns[]");
    expect(offered).toContain("x");
    expect(offered).toContain("label");
  });
});

describe("place", () => {
  it("puts a violation on the field it names", () => {
    const placed = place(
      refused([{ field: "fields[0].type", description: "unknown type" }]),
      FORM
    );

    expect(placed.at.get("fields[0].type")).toBe("unknown type");
    expect(placed.loose).toEqual([]);
  });

  // The service checks more than the form offers; a dropped one would leave a
  // refusal with nothing said about it.
  it("keeps a violation the form cannot show, named", () => {
    const placed = place(
      refused([{ field: "row_identity[3]", description: "no such field" }]),
      FORM
    );

    expect(placed.at.size).toBe(0);
    expect(placed.loose).toEqual(["row_identity[3]: no such field"]);
  });

  it("keeps a violation that names nothing", () => {
    const placed = place(
      refused([{ description: "a dataset needs at least one field" }]),
      FORM
    );

    expect(placed.loose).toEqual(["a dataset needs at least one field"]);
  });

  // One field, two complaints: the first sits on it and the second is still
  // said, rather than overwriting it or disappearing.
  it("shows the second complaint about one field beside the form", () => {
    const placed = place(
      refused([
        { field: "title", description: "must not be empty" },
        { field: "title", description: "too long" },
      ]),
      FORM
    );

    expect(placed.at.get("title")).toBe("must not be empty");
    expect(placed.loose).toEqual(["title: too long"]);
  });

  it("falls back to the plain refusal when no violation was named", () => {
    const placed = place(refused([], "the dataset is being removed"), FORM);

    expect(placed.loose).toEqual(["the dataset is being removed"]);
  });

  it("leaves the plain refusal out once a violation has been placed", () => {
    const placed = place(
      refused([{ field: "title", description: "required" }], "invalid body"),
      FORM
    );

    expect(placed.loose).toEqual([]);
  });

  it("says nothing about an error the service did not send", () => {
    const placed = place(new Error("network down"), FORM);

    expect(placed.at.size).toBe(0);
    expect(placed.loose).toEqual([]);
  });
});
