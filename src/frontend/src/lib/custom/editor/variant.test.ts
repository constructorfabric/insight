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
});
