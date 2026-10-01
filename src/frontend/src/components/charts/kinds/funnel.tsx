import { Funnel, FunnelChart, Tooltip } from "recharts";

import type { CategoryWidget, MetricResult } from "@/api/custom-client";
import { groupedNumber, unitFor } from "@/components/custom/chart-format";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

import { categoryRows } from "../adapters/category";
import { cut, tooltipContent } from "../chart-style";
import { KindChart, KindFigure } from "../chrome";
import { seriesColor } from "../palette";

export function FunnelKind({
  widget,
  result,
}: {
  widget: CategoryWidget;
  result: MetricResult;
}) {
  const unit = unitFor(result.percents, widget.value);
  const stages = categoryRows(result, widget.label, widget.value, {
    positiveOnly: true,
    order: "desc",
  }).map((stage, index) => ({ ...stage, fill: seriesColor(index) }));

  return (
    <KindFigure kind="funnel">
      <div className="grid h-full min-h-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-4">
        <KindChart>
          <FunnelChart margin={{ top: 12, right: 4, bottom: 12, left: 4 }}>
            <Tooltip content={tooltipContent({ unit, hideLabel: true })} />
            <Funnel
              data={stages}
              dataKey="value"
              nameKey="label"
              lastShapeType="rectangle"
              isAnimationActive={false}
            />
          </FunnelChart>
        </KindChart>
        <ol
          aria-label="Stages"
          className="flex max-h-full min-w-0 flex-col gap-1.5 overflow-y-auto"
        >
          {stages.map((stage) => (
            <li
              key={stage.label}
              className="flex items-center justify-between gap-3"
            >
              <span
                className={cn(TEXT_LABEL, "flex min-w-0 items-center gap-1.5")}
              >
                <i
                  aria-hidden="true"
                  className="size-2 shrink-0 rounded-full"
                  style={{ backgroundColor: stage.fill }}
                />
                <span title={stage.label}>{cut(stage.label, 16)}</span>
              </span>
              <strong className="text-sm font-semibold tabular-nums">
                {groupedNumber(stage.value, unit)}
              </strong>
            </li>
          ))}
        </ol>
      </div>
    </KindFigure>
  );
}
