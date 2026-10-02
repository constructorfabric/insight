import { describe, expect, it } from "vitest";

import type { Widget } from "@/api/custom-client";

import {
  isWidgetKind,
  WIDGET_KINDS,
  widgetFields,
  widgetLayout,
} from "./widget-columns";

describe("widgetFields", () => {
  it.each<[Widget, [string, string][]]>([
    [
      { type: "table", metric: "m", columns: ["a", "b"] },
      [
        ["column", "a"],
        ["column", "b"],
      ],
    ],
    [
      {
        type: "line",
        metric: "m",
        x: "day",
        y: "n",
        series: "repo",
        target: "goal",
      },
      [
        ["x", "day"],
        ["y", "n"],
        ["series", "repo"],
        ["target", "goal"],
      ],
    ],
    [
      { type: "bar", metric: "m", x: "day", y: "n" },
      [
        ["x", "day"],
        ["y", "n"],
      ],
    ],
    [
      { type: "stat", metric: "m", value: "n", label: "Lines" },
      [["value", "n"]],
    ],
    [
      { type: "donut", metric: "m", label: "repo", value: "n" },
      [
        ["label", "repo"],
        ["value", "n"],
      ],
    ],
    [
      {
        type: "stacked",
        metric: "m",
        label: "day",
        value: "n",
        series: "repo",
      },
      [
        ["label", "day"],
        ["value", "n"],
        ["series", "repo"],
      ],
    ],
    [
      { type: "composed", metric: "m", x: "day", y: "n", y2: "rate" },
      [
        ["x", "day"],
        ["y", "n"],
        ["y2", "rate"],
      ],
    ],
    [
      { type: "bubble", metric: "m", x: "a", y: "b", size: "c", series: "d" },
      [
        ["x", "a"],
        ["y", "b"],
        ["size", "c"],
        ["series", "d"],
      ],
    ],
    [
      { type: "radar", metric: "m", label: "area", value: "score" },
      [
        ["label", "area"],
        ["value", "score"],
      ],
    ],
    [
      { type: "radial", metric: "m", value: "done", max: "goal" },
      [
        ["value", "done"],
        ["max", "goal"],
      ],
    ],
    [
      { type: "heatmap", metric: "m", x: "day", value: "n" },
      [
        ["x", "day"],
        ["value", "n"],
      ],
    ],
    [
      { type: "pulse", metric: "m", x: "day", y: "n" },
      [
        ["x", "day"],
        ["y", "n"],
      ],
    ],
  ])(
    "names every column a widget draws, in the order it reads them: %o",
    (widget, fields) => {
      expect(
        widgetFields(widget).map(({ field, column }) => [field, column])
      ).toEqual(fields);
    }
  );
});

describe("isWidgetKind", () => {
  it("knows the nineteen kinds the service accepts", () => {
    expect(WIDGET_KINDS).toHaveLength(19);
    expect(WIDGET_KINDS.every(isWidgetKind)).toBe(true);
  });

  it.each(["sankey", "", undefined, 3])("refuses %j", (kind) => {
    expect(isWidgetKind(kind)).toBe(false);
  });
});

describe("widgetFields over a stored kind this build does not know", () => {
  it("names no columns rather than throwing", () => {
    expect(widgetFields({ type: "sankey", metric: "m" } as never)).toEqual([]);
  });
});

describe("widgetLayout", () => {
  it.each<[Widget, { tall: boolean; fullRow: boolean }]>([
    [
      { type: "table", metric: "m", columns: ["a"] },
      { tall: true, fullRow: true },
    ],
    [
      { type: "bar", metric: "m", x: "day", y: "n" },
      { tall: false, fullRow: false },
    ],
    [
      { type: "stat", metric: "m", value: "n" },
      { tall: false, fullRow: false },
    ],
  ])("lays out %o", (widget, layout) => {
    expect(widgetLayout(widget)).toEqual(layout);
  });
});
