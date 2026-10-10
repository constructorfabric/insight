import { useId } from "react";
import { Area, AreaChart } from "recharts";

import type { MetricResult, SeriesWidget } from "@/api/custom-client";
import { unitFor } from "@/components/custom/chart-format";

import { seriesRows } from "../adapters/series";
import { AreaGradient, KindChart, KindFigure } from "../chrome";
import { colorKeys } from "../palette";
import { CURVE } from "./curve";
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
              <AreaGradient
                key={key}
                id={`${id}-${key}`}
                color={color}
                opacity={index === 0 ? 0.28 : 0.2}
              />
            ))}
          </defs>
          <SeriesAxes
            categories={rows.map((row) => row.x)}
            unit={unitFor(result.percents, widget.y)}
          />
          {colored.map(({ key, label, color }) => (
            <Area
              {...CURVE}
              key={key}
              dataKey={key}
              name={label}
              stroke={color}
              fill={`url(#${id}-${key})`}
            />
          ))}
        </AreaChart>
      </KindChart>
    </KindFigure>
  );
}
