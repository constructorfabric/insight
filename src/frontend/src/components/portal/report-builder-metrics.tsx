import type { Dispatch, SetStateAction } from "react";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { MetricDefinition } from "@/api/metric-definitions-client";
import type { MetricFamily } from "@/lib/reports/families";
import { cn } from "@/lib/utils";
import { TEXT_EYEBROW } from "@/lib/type-scale";

export interface OfferedReportMetric extends MetricDefinition {
  reason: string | null;
}

interface ReportMetricPickerProps {
  families: MetricFamily<OfferedReportMetric>[];
  selected: string[];
  setSelected: Dispatch<SetStateAction<string[]>>;
}

export function ReportMetricPicker({
  families,
  selected,
  setSelected,
}: ReportMetricPickerProps) {
  if (families.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        No metrics are available for this subject.
      </p>
    );
  }

  return (
    <TooltipProvider delay={300}>
      <div className="flex flex-col gap-4">
        {families.map((family) => (
          <MetricFamily
            key={family.family}
            family={family}
            selected={selected}
            setSelected={setSelected}
          />
        ))}
      </div>
    </TooltipProvider>
  );
}

function MetricFamily({
  family,
  selected,
  setSelected,
}: {
  family: MetricFamily<OfferedReportMetric>;
  selected: string[];
  setSelected: Dispatch<SetStateAction<string[]>>;
}) {
  const selectable = family.metrics
    .filter((metric) => metric.reason == null)
    .map((metric) => metric.metric_key);
  const allPicked =
    selectable.length > 0 && selectable.every((key) => selected.includes(key));

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <span className={TEXT_EYEBROW}>{family.name}</span>
        {selectable.length > 0 ? (
          <Button
            type="button"
            variant="ghost"
            size="xs"
            onClick={() =>
              setSelected((current) =>
                allPicked
                  ? current.filter((key) => !selectable.includes(key))
                  : [...new Set([...current, ...selectable])]
              )
            }
          >
            {allPicked ? "None" : "All"}
          </Button>
        ) : null}
      </div>
      <div className="grid grid-cols-1 gap-1 sm:grid-cols-2 lg:grid-cols-3">
        {family.metrics.map((metric) => {
          const checked = selected.includes(metric.metric_key);
          return (
            <Tooltip key={metric.metric_key}>
              <TooltipTrigger
                render={
                  <label
                    htmlFor={`report-${metric.metric_key}`}
                    title={metric.reason ?? undefined}
                    className={cn(
                      "flex items-center gap-2 rounded-sm px-1 py-1 text-start text-sm",
                      metric.reason
                        ? "text-muted-foreground"
                        : "cursor-pointer hover:bg-muted"
                    )}
                  >
                    <Checkbox
                      id={`report-${metric.metric_key}`}
                      checked={checked}
                      disabled={metric.reason != null}
                      onCheckedChange={() =>
                        setSelected((current) =>
                          checked
                            ? current.filter((key) => key !== metric.metric_key)
                            : [...current, metric.metric_key]
                        )
                      }
                    />
                    {metric.label}
                  </label>
                }
              />
              <TooltipContent
                side="top"
                className="max-w-xs text-xs leading-relaxed"
              >
                {metric.reason ??
                  metric.description ??
                  metric.explanation ??
                  metric.label}
              </TooltipContent>
            </Tooltip>
          );
        })}
      </div>
    </div>
  );
}
