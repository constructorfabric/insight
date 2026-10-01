import {
  Bar,
  BarChart,
  CartesianGrid,
  LabelList,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import type { CategoryWidget, MetricResult } from "@/api/custom-client";
import {
  compactNumber,
  groupedNumber,
  unitFor,
} from "@/components/custom/chart-format";

import { categoryRows } from "../adapters/category";
import { AXIS_TICK, GRID_STROKE, tooltipContent } from "../chart-style";
import { CategoryTick, KindChart, KindFigure } from "../chrome";
import { seriesColor } from "../palette";

export const RANKED_LIMIT = 10;

export function RankedKind({
  widget,
  result,
}: {
  widget: CategoryWidget;
  result: MetricResult;
}) {
  const rows = categoryRows(result, widget.label, widget.value, {
    order: "desc",
    limit: RANKED_LIMIT,
  });
  const unit = unitFor(result.percents, widget.value);

  return (
    <KindFigure kind="ranked">
      <KindChart>
        <BarChart
          data={rows}
          layout="vertical"
          margin={{ top: 4, right: 40, bottom: 0, left: 8 }}
        >
          <CartesianGrid stroke={GRID_STROKE} horizontal={false} />
          <XAxis
            type="number"
            tick={AXIS_TICK}
            tickLine={false}
            axisLine={false}
            tickFormatter={(value) => compactNumber(value, unit)}
          />
          <YAxis
            type="category"
            dataKey="label"
            interval={0}
            tick={<CategoryTick />}
            tickLine={false}
            axisLine={false}
            width={96}
          />
          <Tooltip content={tooltipContent({ unit })} />
          <Bar
            dataKey="value"
            name={widget.value}
            fill={seriesColor(0)}
            radius={[0, 5, 5, 0]}
            maxBarSize={22}
            isAnimationActive={false}
          >
            <LabelList
              dataKey="value"
              position="right"
              formatter={(value) => groupedNumber(value, unit)}
              fill="var(--muted-foreground)"
              fontSize={10}
            />
          </Bar>
        </BarChart>
      </KindChart>
    </KindFigure>
  );
}
