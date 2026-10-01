import type { CSSProperties, ReactNode } from "react";
import { ChartContainer } from "@gears-frontx/ui-kit";
import type { DotItemDotProps } from "recharts";

import { AXIS_TICK, LABEL_CHARS, cut } from "./chart-style";
import { WidgetLegend, type LegendItem } from "./widget-legend";

const CONTAINER_STYLE: CSSProperties = {
  width: "100%",
  height: "100%",
  minWidth: 0,
  minHeight: 0,
  aspectRatio: "auto",
  flex: "1 1 0",
};

export function KindFigure({
  kind,
  legend = [],
  children,
}: {
  kind: string;
  legend?: LegendItem[];
  children: ReactNode;
}) {
  return (
    <figure
      aria-label={`${kind} chart`}
      className="m-0 flex h-full min-h-0 w-full min-w-0 flex-col"
    >
      {children}
      <WidgetLegend items={legend} />
    </figure>
  );
}

export function KindChart({
  children,
}: {
  children: Parameters<typeof ChartContainer>[0]["children"];
}) {
  return (
    <ChartContainer config={{}} style={CONTAINER_STYLE}>
      {children}
    </ChartContainer>
  );
}

export function CategoryTick({
  x,
  y,
  payload,
  anchor = "end",
  chars = LABEL_CHARS,
  angle,
  format,
}: {
  x?: number | string;
  y?: number | string;
  payload?: { value?: unknown };
  anchor?: "start" | "middle" | "end";
  chars?: number;
  angle?: number;
  format?: (value: unknown) => string;
}) {
  const full = String(payload?.value ?? "");

  return (
    <text
      x={x}
      y={y}
      dy={4}
      textAnchor={anchor}
      transform={angle ? `rotate(${angle}, ${x}, ${y})` : undefined}
      fill={AXIS_TICK.fill}
      fontSize={AXIS_TICK.fontSize}
    >
      <title>{full}</title>
      {format ? format(payload?.value) : cut(full, chars)}
    </text>
  );
}

export function IsolatedDot({
  cx,
  cy,
  index,
  points,
  stroke,
  value,
}: DotItemDotProps) {
  if (value == null || cx == null || cy == null) return null;

  const isolated =
    points[index - 1]?.value == null && points[index + 1]?.value == null;

  return isolated ? <circle cx={cx} cy={cy} r={3} fill={stroke} /> : null;
}
