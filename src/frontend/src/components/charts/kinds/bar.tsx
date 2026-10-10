import { Bar, BarChart, LabelList } from "recharts";

import type { MetricResult, SeriesWidget } from "@/api/custom-client";
import {
  compactNumber,
  toNumber,
  unitFor,
} from "@/components/custom/chart-format";

import {
  rowTotal,
  seriesRows,
  type SeriesKey,
  type SeriesRow,
} from "../adapters/series";
import { EVERY_TICK_LIMIT, VALUE_LABEL } from "../chart-style";
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
  const labelled = rows.length <= EVERY_TICK_LIMIT;
  const tops = new Map(rows.map((row) => [row.x, stackTop(row, keys)]));

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
              {labelled ? (
                <LabelList
                  {...VALUE_LABEL}
                  valueAccessor={(entry: {
                    payload?: SeriesRow;
                    value?: unknown;
                  }) => {
                    if (!stacked) return toNumber(entry.value);

                    const top = tops.get(entry.payload?.x);
                    return top?.key === key ? top.total : undefined;
                  }}
                  formatter={(value) => compactNumber(value, unit)}
                />
              ) : null}
            </Bar>
          ))}
        </BarChart>
      </KindChart>
    </KindFigure>
  );
}

function stackTop(
  row: SeriesRow,
  keys: SeriesKey[]
): { key: string | undefined; total: number } {
  const drawn = keys.filter(({ key }) => toNumber(row[key]) !== null);

  return { key: drawn.at(-1)?.key, total: rowTotal(row, drawn) };
}
