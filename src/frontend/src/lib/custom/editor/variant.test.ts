import { describe, expect, it } from "vitest";

import type { Field } from "./describe";
import { switched, variantOf, type VariantsShape } from "./variant";

const METRIC: Field = {
  name: "metric",
  label: "Metric",
  shape: { of: "reference", to: "metrics" },
};

const BY_TYPE: VariantsShape = {
  of: "variants",
  recorded: "type",
  variants: {
    table: [
      METRIC,
      { name: "columns", label: "Columns", shape: { of: "text" } },
    ],
    stat: [
      METRIC,
      { name: "value", label: "Value", shape: { of: "text" } },
      { name: "label", label: "Label", shape: { of: "text" } },
    ],
    pie: [
      METRIC,
      { name: "label", label: "Slices", shape: { of: "text" } },
      { name: "value", label: "Value", shape: { of: "text" } },
    ],
  },
};

/**
 * A shape whose variants share a property name but read it differently: the
 * metric's own, where a source names either a declared field or a column.
 */
const BY_SOURCE: VariantsShape = {
  of: "variants",
  variants: {
    dataset: [
      { name: "dataset", label: "Dataset", shape: { of: "text" } },
      {
        name: "fields",
        label: "Fields",
        shape: {
          of: "list",
          entryLabel: "field",
          entry: {
            of: "record",
            fields: [
              { name: "field", label: "Field", shape: { of: "text" } },
              { name: "as_name", label: "Called", shape: { of: "text" } },
            ],
          },
        },
      },
      {
        name: "time",
        label: "Window by",
        shape: {
          of: "record",
          fields: [{ name: "field", label: "Date", shape: { of: "text" } }],
        },
      },
    ],
    table: [
      { name: "table", label: "Table", shape: { of: "text" } },
      {
        name: "fields",
        label: "Fields",
        shape: {
          of: "list",
          entryLabel: "field",
          entry: {
            of: "record",
            fields: [
              { name: "column", label: "Column", shape: { of: "text" } },
              { name: "as_name", label: "Called", shape: { of: "text" } },
            ],
          },
        },
      },
      {
        name: "time",
        label: "Window by",
        shape: {
          of: "record",
          fields: [{ name: "column", label: "Date", shape: { of: "text" } }],
        },
      },
    ],
  },
};

const BY_CARRIED: VariantsShape = {
  of: "variants",
  variants: {
    widget: [{ name: "widget", label: "Widget", shape: { of: "text" } }],
    heading: [{ name: "heading", label: "Heading", shape: { of: "text" } }],
  },
};

describe("variantOf", () => {
  it.each([
    [{ type: "table" }, "table"],
    [{ type: 7 }, ""],
    [{}, ""],
  ])("reads the recorded choice from %j", (record, expected) => {
    expect(variantOf(BY_TYPE, record)).toBe(expected);
  });

  it.each([
    [{ heading: "Flow" }, "heading"],
    [{ widget: "" }, "widget"],
    [{ title: "x" }, ""],
  ])("tells %j apart by what it carries", (record, expected) => {
    expect(variantOf(BY_CARRIED, record)).toBe(expected);
  });
});

describe("switched", () => {
  // A chart's metric is the same metric whatever it is drawn as.
  it("keeps what the incoming variant also asks for", () => {
    const record = {
      type: "table",
      metric: "commits",
      columns: "day",
      title: "T",
    };

    expect(switched(BY_TYPE, record, "stat")).toEqual({
      type: "stat",
      metric: "commits",
      title: "T",
    });
  });

  it("keeps fields two variants share, under the new variant", () => {
    const record = {
      type: "stat",
      metric: "m",
      value: "total",
      label: "Total",
    };

    expect(switched(BY_TYPE, record, "pie")).toEqual({
      type: "pie",
      metric: "m",
      value: "total",
      label: "Total",
    });
  });

  it("drops what only the outgoing variant asked for", () => {
    const record = { type: "table", metric: "m", columns: "day" };

    expect(switched(BY_TYPE, record, "stat")).not.toHaveProperty("columns");
  });

  it("carries the new kind's own property when nothing records the choice", () => {
    expect(switched(BY_CARRIED, { widget: "chart" }, "heading")).toEqual({
      heading: "",
    });
  });

  it("leaves a record no variant at all when the choice is cleared", () => {
    expect(
      switched(BY_TYPE, { type: "table", metric: "m", title: "T" }, "")
    ).toEqual({
      title: "T",
    });
  });

  // A metric keeps the columns it produces when its source changes, but what
  // each one read belongs to the source it was written for. Left behind, a
  // stale read is invisible in the editor and refused on sending.
  it("clears what only the outgoing shape could read, however deep it sits", () => {
    const record = {
      dataset: "commits",
      fields: [
        { field: "actor", as_name: "actor" },
        { field: "lines", as_name: "total" },
      ],
      time: { field: "day" },
      group_by: ["actor"],
    };

    expect(switched(BY_SOURCE, record, "table")).toEqual({
      table: "",
      fields: [{ as_name: "actor" }, { as_name: "total" }],
      group_by: ["actor"],
    });
  });

  // The other way round is the same rule, and a property no variant knows is
  // the document's own either way.
  it("clears a column when a metric is turned back over a dataset", () => {
    const record = {
      table: "silver.fct_commit",
      fields: [{ column: "sha", as_name: "sha" }],
      limit: 10,
    };

    expect(switched(BY_SOURCE, record, "dataset")).toEqual({
      dataset: "",
      fields: [{ as_name: "sha" }],
      limit: 10,
    });
  });

  // A row the reader added is theirs: emptying it would make the list shorten
  // under them.
  it("keeps a row that nothing in it survives", () => {
    const record = { dataset: "commits", fields: [{ field: "actor" }] };

    expect(switched(BY_SOURCE, record, "table")).toEqual({
      table: "",
      fields: [{}],
    });
  });
});
