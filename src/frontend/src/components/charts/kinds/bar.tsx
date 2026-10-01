import { Bar, BarChart, LabelList } from "recharts";

import type { MetricResult, SeriesWidget } from "@/api/custom-client";
import { compactNumber, unitFor } from "@/components/custom/chart-format";

import { toNumber } from "../adapters/cells";
import { seriesRows, type SeriesKey, type SeriesRow } from "../adapters/series";
import { VALUE_LABEL } from "../chart-style";
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
  const unit = unitFor(result.percents, widget.y);

  return (
    <KindFigure kind="bar" legend={colored}>
      <KindChart>
        <BarChart
          data={rows}
          margin={{ top: 20, right: 8, bottom: 0, left: 0 }}
          barGap={5}
        >
          <SeriesAxes
            categories={rows.map((row) => row.x)}
            unit={unit}
            everyTick
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
            >
              <LabelList
                {...VALUE_LABEL}
                valueAccessor={(entry: {
                  payload?: SeriesRow;
                  value?: unknown;
                }) =>
                  stacked
                    ? stackTotal(entry.payload, keys, key)
                    : toNumber(entry.value)
                }
                formatter={(value) => compactNumber(value, unit)}
              />
            </Bar>
          ))}
        </BarChart>
      </KindChart>
    </KindFigure>
  );
}

function stackTotal(
  row: SeriesRow | undefined,
  keys: SeriesKey[],
  key: string
): number | undefined {
  if (!row) return undefined;

  const drawn = keys.filter(({ key: name }) => toNumber(row[name]) !== null);
  if (drawn.at(-1)?.key !== key) return undefined;

  return drawn.reduce(
    (sum, { key: name }) => sum + (toNumber(row[name]) ?? 0),
    0
  );
}
