import { Bar, BarChart } from "recharts";

import type { MetricResult, SeriesWidget } from "@/api/custom-client";
import { unitFor } from "@/components/custom/chart-format";

import { seriesRows } from "../adapters/series";
import { KindChart, KindFigure } from "../chrome";
import { colorKeys } from "../palette";
import { SeriesAxes } from "./series-axes";

const GROUPED_SERIES = 2;

export function BarKind({
  widget,
  result,
}: {
  widget: SeriesWidget;
  result: MetricResult;
}) {
  const { rows, keys } = seriesRows(result, widget.x, widget.y, widget.series);
  const colored = colorKeys(keys);
  const stacked = colored.length > GROUPED_SERIES;
  const top = colored.length - 1;

  return (
    <KindFigure kind="bar" legend={colored}>
      <KindChart>
        <BarChart
          data={rows}
          margin={{ top: 8, right: 8, bottom: 0, left: 0 }}
          barGap={5}
        >
          <SeriesAxes
            categories={rows.map((row) => row.x)}
            unit={unitFor(result.percents, widget.y)}
          />
          {colored.map(({ key, label, color }, index) => (
            <Bar
              key={key}
              dataKey={key}
              name={label}
              fill={color}
              stackId={stacked ? "series" : undefined}
              radius={!stacked || index === top ? [5, 5, 0, 0] : 0}
              maxBarSize={20}
              isAnimationActive={false}
            />
          ))}
        </BarChart>
      </KindChart>
    </KindFigure>
  );
}
