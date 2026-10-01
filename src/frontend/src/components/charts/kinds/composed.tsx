import { Bar, ComposedChart, LabelList, Line, YAxis } from "recharts";

import type { ComposedWidget, MetricResult } from "@/api/custom-client";
import { compactNumber, unitFor } from "@/components/custom/chart-format";

import { seriesRows } from "../adapters/series";
import { AXIS_TICK, VALUE_LABEL } from "../chart-style";
import { IsolatedDot, KindChart, KindFigure } from "../chrome";
import { seriesColor } from "../palette";
import { SeriesAxes } from "./series-axes";

const LINE_KEY = "y2";

export function ComposedKind({
  widget,
  result,
}: {
  widget: ComposedWidget;
  result: MetricResult;
}) {
  const { rows, keys } = seriesRows(result, widget.x, widget.y, undefined, {
    [LINE_KEY]: widget.y2,
  });
  const barKey = keys[0]?.key ?? "s0";
  const barColor = seriesColor(0);
  const lineColor = seriesColor(2);
  const lineUnit = unitFor(result.percents, widget.y2);

  return (
    <KindFigure
      kind="composed"
      legend={[
        { label: widget.y, color: barColor },
        { label: widget.y2, color: lineColor, style: "line" },
      ]}
    >
      <KindChart>
        <ComposedChart
          data={rows}
          margin={{ top: 20, right: 8, bottom: 0, left: 0 }}
        >
          <SeriesAxes
            categories={rows.map((row) => row.x)}
            unit={unitFor(result.percents, widget.y)}
            everyTick
          />
          <YAxis
            yAxisId={LINE_KEY}
            orientation="right"
            tick={AXIS_TICK}
            tickLine={false}
            axisLine={false}
            width={38}
            tickFormatter={(value) => compactNumber(value, lineUnit)}
          />
          <Bar
            dataKey={barKey}
            name={widget.y}
            fill={barColor}
            fillOpacity={0.82}
            radius={[4, 4, 0, 0]}
            maxBarSize={20}
            isAnimationActive={false}
          >
            <LabelList
              {...VALUE_LABEL}
              formatter={(value) =>
                compactNumber(value, unitFor(result.percents, widget.y))
              }
            />
          </Bar>
          <Line
            yAxisId={LINE_KEY}
            type="monotone"
            dataKey={LINE_KEY}
            name={widget.y2}
            stroke={lineColor}
            strokeWidth={2.5}
            dot={IsolatedDot}
            activeDot={{ r: 4 }}
            isAnimationActive={false}
          />
        </ComposedChart>
      </KindChart>
    </KindFigure>
  );
}
