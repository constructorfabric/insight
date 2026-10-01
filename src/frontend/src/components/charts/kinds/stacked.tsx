import { Bar, BarChart, CartesianGrid, Tooltip, XAxis, YAxis } from "recharts";

import type { MetricResult, StackedWidget } from "@/api/custom-client";

import { shareRows } from "../adapters/series";
import { AXIS_TICK, GRID_STROKE, tooltipContent } from "../chart-style";
import { CategoryTick, KindChart, KindFigure } from "../chrome";
import { colorKeys } from "../palette";

const SHARE_TICKS = [0, 25, 50, 75, 100];

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
    widget.series
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
          <CartesianGrid stroke={GRID_STROKE} horizontal={false} />
          <XAxis
            type="number"
            domain={[0, 100]}
            ticks={SHARE_TICKS}
            tickFormatter={(value) => `${value}%`}
            tick={AXIS_TICK}
            tickLine={false}
            axisLine={false}
          />
          <YAxis
            type="category"
            dataKey="x"
            interval={0}
            tick={<CategoryTick />}
            tickLine={false}
            axisLine={false}
            width={80}
          />
          <Tooltip content={tooltipContent({ unit: "%" })} />
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
