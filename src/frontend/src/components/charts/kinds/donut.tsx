import { Label, Pie, PieChart, Tooltip } from "recharts";

import type { CategoryWidget, MetricResult } from "@/api/custom-client";
import { compactNumber, unitFor } from "@/components/custom/chart-format";

import { categoryRows, withOther } from "../adapters/category";
import { OTHER_LABEL } from "../adapters/cells";
import { SERIES_LIMIT } from "../adapters/series";
import { tooltipContent } from "../chart-style";
import { KindChart, KindFigure } from "../chrome";
import { OTHER_COLOR, seriesColor } from "../palette";

export function DonutKind({
  widget,
  result,
}: {
  widget: CategoryWidget;
  result: MetricResult;
}) {
  const donut = widget.type === "donut";
  const unit = unitFor(result.percents, widget.value);
  const slices = withOther(
    categoryRows(result, widget.label, widget.value, { positiveOnly: true }),
    SERIES_LIMIT
  ).map((slice, index) => ({
    ...slice,
    fill: slice.label === OTHER_LABEL ? OTHER_COLOR : seriesColor(index),
  }));
  const total = slices.reduce((sum, slice) => sum + slice.value, 0);

  return (
    <KindFigure
      kind={widget.type}
      legend={slices.map(({ label, fill }) => ({ label, color: fill }))}
    >
      <KindChart>
        <PieChart margin={{ top: 8, right: 8, bottom: 8, left: 8 }}>
          <Tooltip content={tooltipContent({ unit, hideLabel: true })} />
          <Pie
            data={slices}
            dataKey="value"
            nameKey="label"
            cx="50%"
            cy="50%"
            innerRadius={donut ? "61%" : 0}
            outerRadius="82%"
            paddingAngle={2}
            stroke="var(--card)"
            strokeWidth={2}
            cornerRadius={donut ? 4 : 0}
            isAnimationActive={false}
          >
            {donut ? (
              <Label
                value={compactNumber(total, unit)}
                position="center"
                fill="var(--foreground)"
                fontSize={25}
                fontWeight={650}
              />
            ) : null}
          </Pie>
        </PieChart>
      </KindChart>
    </KindFigure>
  );
}
