import { describe, expect, it } from "vitest";

import type { Field } from "./describe";
import { starting } from "./blank";
import { DESCRIPTIONS } from "./kinds";

describe("starting a new definition", () => {
  // A choice made for a reader is a choice they were never offered, so only
  // a kind whose description asks for one opens with it made.
  it.each([
    ["datasets", { source: { kind: "stream" } }],
    ["metrics", {}],
    ["widgets", {}],
    ["dashboards", {}],
  ] as const)("opens a %s as %j", (kind, expected) => {
    expect(starting(DESCRIPTIONS[kind].fields)).toEqual(expected);
  });
});

describe("the variant a new one opens on", () => {
  const variants = (starts: unknown): Field[] => [
    {
      name: "source",
      label: "Over",
      shape: {
        of: "variants",
        recorded: "kind",
        ...(starts === undefined ? {} : { starts }),
        variants: { first: [], second: [] },
      } as Field["shape"],
    },
  ];

  it("is the one the description names, not the one written first", () => {
    expect(starting(variants("second"))).toEqual({ kind: "second" });
  });

  it("is left to the reader where the description names none", () => {
    expect(starting(variants(undefined))).toEqual({});
  });

  // The locator of a dataset's field works this way: a relation holds its
  // values in columns, and a record sent in holds them under a key path.
  it("carries the chosen variant's own property where nothing records it", () => {
    const locator: Field[] = [
      {
        name: "at",
        label: "Where it sits",
        shape: {
          of: "variants",
          starts: "path",
          variants: { path: [], column: [] },
        },
      },
    ];

    expect(starting(locator)).toEqual({ path: "" });
  });

  // A record that seeds nothing is not sent as an empty object: the service
  // reads a property that is there, and `{}` is not nothing.
  it("leaves out a record with no choice of its own to make", () => {
    const empty: Field[] = [
      {
        name: "time",
        label: "Time",
        shape: { of: "record", fields: [{ name: "field", label: "Field", shape: { of: "text" } }] },
      },
    ];

    expect(starting(empty)).toEqual({});
  });
});
