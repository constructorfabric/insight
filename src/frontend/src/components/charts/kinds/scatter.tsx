import {
  CartesianGrid,
  Scatter,
  ScatterChart,
  Tooltip,
  XAxis,
  YAxis,
  ZAxis,
} from "recharts";

import type {
  BubbleWidget,
  MetricResult,
  ScatterWidget,
} from "@/api/custom-client";
import { compactNumber, unitFor } from "@/components/custom/chart-format";

import { pointGroups } from "../adapters/series";
import { AXIS_TICK, GRID_STROKE, tooltipContent } from "../chart-style";
import { KindChart, KindFigure } from "../chrome";
import { colorKeys } from "../palette";

const POINT_RANGE: [number, number] = [60, 60];
const BUBBLE_RANGE: [number, number] = [100, 520];

export function ScatterKind({
  widget,
  result,
}: {
  widget: ScatterWidget | BubbleWidget;
  result: MetricResult;
}) {
  const size = widget.type === "bubble" ? widget.size : undefined;
  const groups = pointGroups(result, widget.x, widget.y, {
    series: widget.series,
    size,
  });
  const colored = colorKeys(groups);
  const xUnit = unitFor(result.percents, widget.x);
  const yUnit = unitFor(result.percents, widget.y);

  return (
    <KindFigure kind={widget.type} legend={colored}>
      <KindChart>
        <ScatterChart margin={{ top: 12, right: 18, bottom: 8, left: 0 }}>
          <CartesianGrid stroke={GRID_STROKE} />
          <XAxis
            type="number"
            dataKey="x"
            name={widget.x}
            tick={AXIS_TICK}
            tickLine={false}
            axisLine={false}
            tickFormatter={(value) => compactNumber(value, xUnit)}
          />
          <YAxis
            type="number"
            dataKey="y"
            name={widget.y}
            tick={AXIS_TICK}
            tickLine={false}
            axisLine={false}
            width={42}
            tickFormatter={(value) => compactNumber(value, yUnit)}
          />
          <ZAxis
            type="number"
            dataKey="size"
            name={size ?? "size"}
            range={size ? BUBBLE_RANGE : POINT_RANGE}
          />
          <Tooltip
            cursor={{ strokeDasharray: "3 3" }}
            content={tooltipContent({ hideLabel: true })}
          />
          {colored.map(({ key, label, color, points }) => (
            <Scatter
              key={key}
              name={label}
              data={points}
              fill={color}
              fillOpacity={size ? 0.7 : 0.78}
              isAnimationActive={false}
            />
          ))}
        </ScatterChart>
      </KindChart>
    </KindFigure>
  );
}
