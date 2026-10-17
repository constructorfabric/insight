import { Bar, BarChart, LabelList } from "recharts";

import type { CategoryWidget, MetricResult } from "@/api/custom-client";
import { groupedNumber, unitFor } from "@/components/custom/chart-format";

import { categoryRows, ranked } from "../adapters/category";
import { KindChart, KindFigure } from "../chrome";
import { seriesColor } from "../palette";
import { CategoryAxes } from "./category-axes";

const RANKED_LIMIT = 10;

export function RankedKind({
  widget,
  result,
}: {
  widget: CategoryWidget;
  result: MetricResult;
}) {
  const rows = ranked(
    categoryRows(result, widget.label, widget.value),
    RANKED_LIMIT
  );
  const unit = unitFor(result.percents, widget.value);

  return (
    <KindFigure kind="ranked">
      <KindChart>
        <BarChart
          data={rows}
          layout="vertical"
          margin={{ top: 4, right: 40, bottom: 0, left: 8 }}
        >
          <CategoryAxes dataKey="label" unit={unit} />
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
