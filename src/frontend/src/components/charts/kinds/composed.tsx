import { Bar, ComposedChart, LabelList, Line, YAxis } from "recharts";

import type { ComposedWidget, MetricResult } from "@/api/custom-client";
import { compactNumber, unitFor } from "@/components/custom/chart-format";

import { pairedRows } from "../adapters/series";
import { AXIS_TICK, EVERY_TICK_LIMIT, VALUE_LABEL } from "../chart-style";
import { KindChart, KindFigure } from "../chrome";
import { seriesColor } from "../palette";
import { CURVE } from "./curve";
import { SeriesAxes } from "./series-axes";

export function ComposedKind({
  widget,
  result,
}: {
  widget: ComposedWidget;
  result: MetricResult;
}) {
  const rows = pairedRows(result, widget.x, widget.y, widget.y2);
  const barColor = seriesColor(0);
  const lineColor = seriesColor(2);
  const barUnit = unitFor(result.percents, widget.y);
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
            unit={barUnit}
            everyTick
          />
          <YAxis
            yAxisId="y2"
            orientation="right"
            tick={AXIS_TICK}
            tickLine={false}
            axisLine={false}
            width={38}
            tickFormatter={(value) => compactNumber(value, lineUnit)}
          />
          <Bar
            dataKey="y"
            name={widget.y}
            fill={barColor}
            fillOpacity={0.82}
            radius={[4, 4, 0, 0]}
            maxBarSize={20}
            isAnimationActive={false}
          >
            {rows.length <= EVERY_TICK_LIMIT ? (
              <LabelList
                {...VALUE_LABEL}
                formatter={(value) => compactNumber(value, barUnit)}
              />
            ) : null}
          </Bar>
          <Line
            {...CURVE}
            yAxisId="y2"
            dataKey="y2"
            name={widget.y2}
            stroke={lineColor}
          />
        </ComposedChart>
      </KindChart>
    </KindFigure>
  );
}
