import {
  PolarAngleAxis,
  PolarGrid,
  PolarRadiusAxis,
  Radar,
  RadarChart,
  Tooltip,
} from "recharts";

import type { MetricResult, RadarWidget } from "@/api/custom-client";
import { unitFor } from "@/components/custom/chart-format";

import { radarRows } from "../adapters/category";
import { AXIS_TICK, cut, tooltipContent } from "../chart-style";
import { KindChart, KindFigure } from "../chrome";
import { seriesColor } from "../palette";
import type { LegendItem } from "../widget-legend";

export function RadarKind({
  widget,
  result,
}: {
  widget: RadarWidget;
  result: MetricResult;
}) {
  const rows = radarRows(result, widget.label, widget.value, widget.target);
  const valueColor = seriesColor(0);
  const targetColor = seriesColor(2);
  const legend: LegendItem[] = [{ label: widget.value, color: valueColor }];
  if (widget.target) {
    legend.push({ label: widget.target, color: targetColor, style: "dashed" });
  }

  return (
    <KindFigure kind="radar" legend={legend}>
      <KindChart>
        <RadarChart data={rows} cx="50%" cy="50%" outerRadius="72%">
          <PolarGrid />
          <PolarAngleAxis
            dataKey="label"
            tick={AXIS_TICK}
            tickFormatter={(value) => cut(value)}
          />
          <PolarRadiusAxis tick={false} axisLine={false} />
          <Tooltip
            content={tooltipContent({
              unit: unitFor(result.percents, widget.value),
            })}
          />
          <Radar
            dataKey="value"
            name={widget.value}
            stroke={valueColor}
            fill={valueColor}
            fillOpacity={0.22}
            strokeWidth={2}
            isAnimationActive={false}
          />
          {widget.target ? (
            <Radar
              dataKey="target"
              name={widget.target}
              stroke={targetColor}
              fill={targetColor}
              fillOpacity={0.05}
              strokeWidth={1.5}
              strokeDasharray="4 4"
              isAnimationActive={false}
            />
          ) : null}
        </RadarChart>
      </KindChart>
    </KindFigure>
  );
}
