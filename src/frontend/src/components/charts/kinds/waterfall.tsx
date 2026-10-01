import {
  Bar,
  BarChart,
  CartesianGrid,
  LabelList,
  Rectangle,
  Tooltip,
  XAxis,
  YAxis,
  type BarShapeProps,
} from "recharts";

import type { CategoryWidget, MetricResult } from "@/api/custom-client";
import { compactNumber, unitFor } from "@/components/custom/chart-format";

import { bridgeRows, type BridgeRow } from "../adapters/bridge";
import {
  AXIS_TICK,
  GRID_STROKE,
  VALUE_LABEL,
  tooltipContent,
} from "../chart-style";
import { CategoryTick, KindChart, KindFigure } from "../chrome";
import { seriesColor } from "../palette";

const KIND_COLOR: Record<BridgeRow["kind"], string> = {
  increase: seriesColor(1),
  decrease: seriesColor(4),
  total: seriesColor(0),
};

export function WaterfallKind({
  widget,
  result,
}: {
  widget: CategoryWidget;
  result: MetricResult;
}) {
  const rows = bridgeRows(result, widget.label, widget.value);
  const unit = unitFor(result.percents, widget.value);

  return (
    <KindFigure
      kind="waterfall"
      legend={[
        { label: "Increase", color: KIND_COLOR.increase },
        { label: "Decrease", color: KIND_COLOR.decrease },
        { label: "Total", color: KIND_COLOR.total },
      ]}
    >
      <KindChart>
        <BarChart
          data={rows}
          margin={{ top: 20, right: 16, bottom: 4, left: 4 }}
        >
          <CartesianGrid stroke={GRID_STROKE} vertical={false} />
          <XAxis
            dataKey="label"
            interval={0}
            tick={<CategoryTick anchor="end" chars={10} angle={-30} />}
            tickLine={false}
            axisLine={false}
            height={58}
          />
          <YAxis
            tick={AXIS_TICK}
            tickLine={false}
            axisLine={false}
            width={48}
            tickFormatter={(value) => compactNumber(value, unit)}
          />
          <Tooltip
            content={tooltipContent({
              unit,
              pick: (row) => row.delta,
            })}
          />
          <Bar
            dataKey={(row: BridgeRow) => [row.low, row.high]}
            name={widget.value}
            radius={4}
            isAnimationActive={false}
            shape={(props: BarShapeProps) => (
              <Rectangle
                {...props}
                fill={KIND_COLOR[(props.payload as BridgeRow).kind]}
              />
            )}
          >
            <LabelList
              {...VALUE_LABEL}
              valueAccessor={(entry: { payload?: BridgeRow }) =>
                entry.payload?.delta
              }
              formatter={(value) => compactNumber(value, unit)}
            />
          </Bar>
        </BarChart>
      </KindChart>
    </KindFigure>
  );
}
