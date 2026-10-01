import { useId } from "react";
import { ArrowDownRight, ArrowUpRight } from "lucide-react";
import { Area, AreaChart } from "recharts";

import type { MetricResult, PulseWidget } from "@/api/custom-client";
import {
  groupedNumber,
  shortDate,
  unitFor,
} from "@/components/custom/chart-format";
import { TEXT_FIGURE, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

import { pulseSummary } from "../adapters/pulse";
import { KindChart, KindFigure } from "../chrome";
import { seriesColor } from "../palette";

export function PulseKind({
  widget,
  result,
}: {
  widget: PulseWidget;
  result: MetricResult;
}) {
  const id = useId().replace(/:/g, "");
  const unit = unitFor(result.percents, widget.y);
  const summary = pulseSummary(result, widget.y);
  const color = seriesColor(0);
  const at = result.columns.indexOf(widget.x);
  const first = result.rows[0]?.[at];
  const last = result.rows.at(-1)?.[at];
  const rising = (summary.change ?? 0) >= 0;
  const Arrow = rising ? ArrowUpRight : ArrowDownRight;

  return (
    <KindFigure kind="pulse">
      <div className="flex h-full min-h-0 flex-col gap-1">
        <strong className={cn(TEXT_FIGURE, "tracking-tight")}>
          {summary.latest === null ? "—" : groupedNumber(summary.latest, unit)}
        </strong>
        {summary.change === null ? null : (
          <span className={cn(TEXT_LABEL, "flex items-center gap-1")}>
            <Arrow aria-hidden="true" className="size-3.5 text-foreground" />
            <span className="font-semibold text-foreground">
              {groupedNumber(Math.abs(summary.change), "%")}
            </span>
            <span>since {shortDate(first)}</span>
          </span>
        )}
        <div className="flex min-h-0 flex-1 flex-col">
          <KindChart>
            <AreaChart
              data={summary.points.map((value) => ({ value }))}
              margin={{ top: 12, right: 0, bottom: 0, left: 0 }}
            >
              <defs>
                <linearGradient id={`${id}-pulse`} x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor={color} stopOpacity={0.24} />
                  <stop offset="95%" stopColor={color} stopOpacity={0.01} />
                </linearGradient>
              </defs>
              <Area
                type="monotone"
                dataKey="value"
                stroke={color}
                strokeWidth={2.5}
                fill={`url(#${id}-pulse)`}
                isAnimationActive={false}
              />
            </AreaChart>
          </KindChart>
          <div className={cn(TEXT_LABEL, "flex justify-between font-normal")}>
            <span>{shortDate(first)}</span>
            <span>{shortDate(last)}</span>
          </div>
        </div>
      </div>
    </KindFigure>
  );
}
