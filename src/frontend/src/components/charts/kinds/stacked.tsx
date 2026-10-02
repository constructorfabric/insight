import { Bar, BarChart } from "recharts";

import type { MetricResult, StackedWidget } from "@/api/custom-client";

import { shareRows } from "../adapters/series";
import { KindChart, KindFigure } from "../chrome";
import { colorKeys } from "../palette";
import { CategoryAxes } from "./category-axes";

const STACKED_LABELS = 8;

export function StackedKind({
  widget,
  result,
}: {
  widget: StackedWidget;
  result: MetricResult;
}) {
  const { rows, keys } = shareRows(
    result,
    widget.label,
    widget.value,
    widget.series,
    STACKED_LABELS
  );
  const colored = colorKeys(keys);
  const last = colored.length - 1;

  return (
    <KindFigure kind="stacked" legend={colored}>
      <KindChart>
        <BarChart
          data={rows}
          layout="vertical"
          margin={{ top: 8, right: 8, bottom: 0, left: 0 }}
        >
          <CategoryAxes dataKey="x" share />
          {colored.map(({ key, label, color }, index) => (
            <Bar
              key={key}
              dataKey={key}
              name={label}
              stackId="share"
              fill={color}
              radius={[
                index === 0 ? 5 : 0,
                index === last ? 5 : 0,
                index === last ? 5 : 0,
                index === 0 ? 5 : 0,
              ]}
              maxBarSize={26}
              isAnimationActive={false}
            />
          ))}
        </BarChart>
      </KindChart>
    </KindFigure>
  );
}
