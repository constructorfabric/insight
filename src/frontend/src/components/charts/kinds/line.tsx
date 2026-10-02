import { Line, LineChart } from "recharts";

import type { LineWidget, MetricResult } from "@/api/custom-client";
import { unitFor } from "@/components/custom/chart-format";

import { seriesRows } from "../adapters/series";
import { IsolatedDot, KindChart, KindFigure } from "../chrome";
import { colorKeys, seriesColor } from "../palette";
import type { LegendItem } from "../widget-legend";
import { SeriesAxes } from "./series-axes";

export function LineKind({
  widget,
  result,
}: {
  widget: LineWidget;
  result: MetricResult;
}) {
  const { rows, keys } = seriesRows(
    result,
    widget.x,
    widget.y,
    widget.series,
    widget.target
  );
  const colored = colorKeys(keys);
  const targetColor = seriesColor(colored.length);
  const legend: LegendItem[] = widget.target
    ? [
        ...colored,
        { label: widget.target, color: targetColor, style: "dashed" },
      ]
    : colored;

  return (
    <KindFigure kind="line" legend={legend}>
      <KindChart>
        <LineChart
          data={rows}
          margin={{ top: 8, right: 12, bottom: 0, left: 0 }}
        >
          <SeriesAxes
            categories={rows.map((row) => row.x)}
            unit={unitFor(result.percents, widget.y)}
          />
          {colored.map(({ key, label, color }) => (
            <Line
              key={key}
              type="monotone"
              dataKey={key}
              name={label}
              stroke={color}
              strokeWidth={2.5}
              dot={IsolatedDot}
              activeDot={{ r: 4 }}
              isAnimationActive={false}
            />
          ))}
          {widget.target ? (
            <Line
              type="monotone"
              dataKey="target"
              name={widget.target}
              stroke={targetColor}
              strokeWidth={2}
              strokeDasharray="5 5"
              dot={false}
              activeDot={{ r: 4 }}
              isAnimationActive={false}
            />
          ) : null}
        </LineChart>
      </KindChart>
    </KindFigure>
  );
}
