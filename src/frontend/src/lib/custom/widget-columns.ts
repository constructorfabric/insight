import type { Widget, WidgetKind } from "@/api/custom-client";

export const WIDGET_KINDS: readonly WidgetKind[] = [
  "table",
  "line",
  "bar",
  "area",
  "stat",
  "pie",
  "donut",
  "ranked",
  "treemap",
  "funnel",
  "waterfall",
  "stacked",
  "composed",
  "scatter",
  "bubble",
  "radar",
  "radial",
  "heatmap",
  "pulse",
];

export interface WidgetField {
  field: string;
  column: string;
}

export function widgetFields(widget: Widget): WidgetField[] {
  switch (widget.type) {
    case "table":
      return widget.columns.map((column) => ({ field: "column", column }));
    case "line":
      return named({
        x: widget.x,
        y: widget.y,
        series: widget.series,
        target: widget.target,
      });
    case "bar":
    case "area":
    case "scatter":
      return named({ x: widget.x, y: widget.y, series: widget.series });
    case "stat":
      return named({ value: widget.value });
    case "pie":
    case "donut":
    case "ranked":
    case "treemap":
    case "funnel":
    case "waterfall":
      return named({ label: widget.label, value: widget.value });
    case "stacked":
      return named({
        label: widget.label,
        value: widget.value,
        series: widget.series,
      });
    case "composed":
      return named({ x: widget.x, y: widget.y, y2: widget.y2 });
    case "bubble":
      return named({
        x: widget.x,
        y: widget.y,
        size: widget.size,
        series: widget.series,
      });
    case "radar":
      return named({
        label: widget.label,
        value: widget.value,
        target: widget.target,
      });
    case "radial":
      return named({ value: widget.value, max: widget.max });
    case "heatmap":
      return named({ x: widget.x, value: widget.value });
    case "pulse":
      return named({ x: widget.x, y: widget.y });
    default:
      return [];
  }
}

export function isWidgetKind(type: unknown): type is WidgetKind {
  return WIDGET_KINDS.includes(type as WidgetKind);
}

export function widgetColumns(widget: Widget): string[] {
  return widgetFields(widget).map(({ column }) => column);
}

function named(fields: Record<string, string | undefined>): WidgetField[] {
  return Object.entries(fields).flatMap(([field, column]) =>
    column ? [{ field, column }] : []
  );
}
