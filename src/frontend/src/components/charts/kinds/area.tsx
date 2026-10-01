import { useId } from "react";
import { Area, AreaChart } from "recharts";

import type { MetricResult, SeriesWidget } from "@/api/custom-client";
import { unitFor } from "@/components/custom/chart-format";

import { seriesRows } from "../adapters/series";
import { IsolatedDot, KindChart, KindFigure } from "../chrome";
import { colorKeys } from "../palette";
import { SeriesAxes } from "./series-axes";

export function AreaKind({
  widget,
  result,
}: {
  widget: SeriesWidget;
  result: MetricResult;
}) {
  const id = useId().replace(/:/g, "");
  const { rows, keys } = seriesRows(result, widget.x, widget.y, widget.series);
  const colored = colorKeys(keys);

  return (
    <KindFigure kind="area" legend={colored}>
      <KindChart>
        <AreaChart
          data={rows}
          margin={{ top: 8, right: 8, bottom: 0, left: 0 }}
        >
          <defs>
            {colored.map(({ key, color }, index) => (
              <linearGradient
                key={key}
                id={`${id}-${key}`}
                x1="0"
                y1="0"
                x2="0"
                y2="1"
              >
                <stop
                  offset="0%"
                  stopColor={color}
                  stopOpacity={index === 0 ? 0.28 : 0.2}
                />
                <stop offset="95%" stopColor={color} stopOpacity={0.01} />
              </linearGradient>
            ))}
          </defs>
          <SeriesAxes
            categories={rows.map((row) => row.x)}
            unit={unitFor(result.percents, widget.y)}
          />
          {colored.map(({ key, label, color }) => (
            <Area
              key={key}
              type="monotone"
              dataKey={key}
              name={label}
              stroke={color}
              fill={`url(#${id}-${key})`}
              strokeWidth={2.5}
              dot={IsolatedDot}
              activeDot={{ r: 4 }}
              isAnimationActive={false}
            />
          ))}
        </AreaChart>
      </KindChart>
    </KindFigure>
  );
}
