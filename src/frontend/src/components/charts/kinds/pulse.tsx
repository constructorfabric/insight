import { useId } from "react";
import { ArrowDownRight, ArrowUpRight } from "lucide-react";
import { Area, AreaChart } from "recharts";

import type { MetricResult, PulseWidget } from "@/api/custom-client";
import {
  groupedNumber,
  shortDate,
  unitFor,
} from "@/components/custom/chart-format";
import { formatTileDelta } from "@/lib/metrics/delta";
import { TEXT_FIGURE, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

import { pulseSummary } from "../adapters/pulse";
import { AreaGradient, KindChart, KindFigure } from "../chrome";
import { seriesColor } from "../palette";

export function PulseKind({
  widget,
  result,
}: {
  widget: PulseWidget;
  result: MetricResult;
}) {
  const id = `${useId().replace(/:/g, "")}-pulse`;
  const unit = unitFor(result.percents, widget.y);
  const summary = pulseSummary(result, widget.x, widget.y);
  const color = seriesColor(0);
  const change =
    summary.change === null
      ? null
      : formatTileDelta({ kind: "percent_change", value: summary.change });
  const Arrow = (summary.change ?? 0) >= 0 ? ArrowUpRight : ArrowDownRight;

  return (
    <KindFigure kind="pulse">
      <div className="flex h-full min-h-0 flex-col gap-1">
        <strong className={cn(TEXT_FIGURE, "tracking-tight")}>
          {summary.latest === null ? "—" : groupedNumber(summary.latest, unit)}
        </strong>
        {change === null ? null : (
          <span className={cn(TEXT_LABEL, "flex items-center gap-1")}>
            <Arrow aria-hidden="true" className="size-3.5 text-foreground" />
            <span className="font-semibold text-foreground">{change}</span>
            <span>since {shortDate(summary.since)}</span>
          </span>
        )}
        <div className="flex min-h-0 flex-1 flex-col">
          <KindChart>
            <AreaChart
              data={summary.points.map((value) => ({ value }))}
              margin={{ top: 12, right: 0, bottom: 0, left: 0 }}
            >
              <defs>
                <AreaGradient id={id} color={color} opacity={0.24} />
              </defs>
              <Area
                type="monotone"
                dataKey="value"
                stroke={color}
                strokeWidth={2.5}
                fill={`url(#${id})`}
                isAnimationActive={false}
              />
            </AreaChart>
          </KindChart>
          <div className={cn(TEXT_LABEL, "flex justify-between font-normal")}>
            <span>{shortDate(summary.since)}</span>
            <span>{shortDate(summary.until)}</span>
          </div>
        </div>
      </div>
    </KindFigure>
  );
}
