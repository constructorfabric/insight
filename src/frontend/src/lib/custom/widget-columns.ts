import type { Widget, WidgetKind } from "@/api/custom-client";

type WidgetOf<K extends WidgetKind> = Widget extends infer W
  ? W extends { type: infer T }
    ? K extends T
      ? W
      : never
    : never
  : never;

type DrawnFields = {
  [K in WidgetKind]: readonly Exclude<
    keyof WidgetOf<K>,
    "type" | "metric" | "title" | "detail"
  >[];
};

const DRAWN: DrawnFields = {
  table: [],
  line: ["x", "y", "series", "target"],
  bar: ["x", "y", "series"],
  area: ["x", "y", "series"],
  stat: ["value"],
  pie: ["label", "value"],
  donut: ["label", "value"],
  ranked: ["label", "value"],
  treemap: ["label", "value"],
  funnel: ["label", "value"],
  waterfall: ["label", "value"],
  stacked: ["label", "value", "series"],
  composed: ["x", "y", "y2"],
  scatter: ["x", "y", "series"],
  bubble: ["x", "y", "size", "series"],
  radar: ["label", "value", "target"],
  radial: ["value", "max"],
  heatmap: ["x", "value"],
  pulse: ["x", "y"],
};

export const WIDGET_KINDS = Object.keys(DRAWN) as WidgetKind[];

interface WidgetField {
  field: string;
  column: string;
}

export function widgetFields(widget: Widget): WidgetField[] {
  if (widget.type === "table") {
    return widget.columns.map((column) => ({ field: "column", column }));
  }
  if (!isWidgetKind(widget.type)) return [];

  const named = widget as unknown as Record<string, unknown>;
  const fields: readonly string[] = DRAWN[widget.type];
  return fields.flatMap((field) => {
    const column = named[field];
    return typeof column === "string" && column ? [{ field, column }] : [];
  });
}

export function widgetLayout(widget: Widget): {
  tall: boolean;
  fullRow: boolean;
} {
  const rows = widget.type === "table";

  return { tall: rows, fullRow: rows };
}

export function isWidgetKind(type: unknown): type is WidgetKind {
  return WIDGET_KINDS.includes(type as WidgetKind);
}

export function widgetColumns(widget: Widget): string[] {
  return widgetFields(widget).map(({ column }) => column);
}
