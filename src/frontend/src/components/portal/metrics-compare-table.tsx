import { Link } from "@tanstack/react-router";
import { ArrowDown, ArrowUp } from "lucide-react";

import type { MetricDirection } from "@/api/metric-results-client";
import {
  MetricHelpTooltip,
  MetricValue,
  UnitNote,
} from "@/components/portal/metrics-table";
import type { MembersGridMember } from "@/components/widgets/dashboard/members-grid";
import { Tooltip, TooltipContent, TooltipTrigger } from "@gears-frontx/ui-kit";
import { formatMetricFigure } from "@/lib/format";
import { metricHelp, type MetricHelpText } from "@/lib/insight/metric-help";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import {
  compareGaps,
  compareReference,
  finiteValue,
  formatGap,
  type CompareReference,
} from "@/lib/portal/metrics-view";
import { directionLabel } from "@/lib/status";
import { cn } from "@/lib/utils";

const STICKY_START = "sticky start-0 z-10 bg-card";
const GAP_SLOT =
  "ms-2 inline-block w-14 text-start text-xs font-normal text-muted-foreground";
const REFERENCE_COLUMNS: readonly {
  key: keyof CompareReference;
  label: string;
  hint: string;
}[] = [
  {
    key: "median",
    label: "Median",
    hint: "Median of the people in this comparison. People with no value are left out.",
  },
  {
    key: "average",
    label: "Average",
    hint: "Average of the people in this comparison. People with no value are left out.",
  },
];

export function MetricsCompareTable({
  people,
  keys,
  byKey,
  helpByKey,
  roles,
}: {
  people: readonly MembersGridMember[];
  keys: readonly string[];
  byKey: ReadonlyMap<string, NormalizedMetricResult>;
  helpByKey: ReadonlyMap<string, MetricHelpText>;
  roles: ReadonlyMap<string, string>;
}) {
  return (
    <div className="max-h-[75svh] overflow-auto">
      <table className="w-full border-separate border-spacing-0 text-sm">
        <caption className="sr-only">Metrics compared side by side</caption>
        <thead>
          <tr>
            <th
              scope="col"
              className={cn(
                STICKY_START,
                "top-0 z-20 border-b px-3 py-2 text-start text-xs font-medium tracking-wider text-muted-foreground uppercase"
              )}
            >
              Metric
            </th>
            {people.map((person) => (
              <th
                key={person.entityId}
                scope="col"
                className="sticky top-0 z-10 border-b bg-card px-3 py-2 text-end font-medium"
              >
                <Link
                  to="/ic/$person/personal"
                  params={{ person: person.entityId }}
                  className="hover:underline"
                >
                  {person.displayName}
                </Link>
                <span aria-hidden className={GAP_SLOT} />
                {roles.get(person.entityId) ? (
                  <span className="block pe-16 text-xs font-normal text-muted-foreground">
                    {roles.get(person.entityId)}
                  </span>
                ) : null}
              </th>
            ))}
            {REFERENCE_COLUMNS.map((column) => (
              <th
                key={column.key}
                scope="col"
                className={cn(
                  "sticky top-0 z-10 border-b bg-muted px-3 py-2 text-end font-medium text-muted-foreground",
                  column.key === "median" && "border-s"
                )}
              >
                <Tooltip>
                  <TooltipTrigger
                    render={
                      <button type="button" className="cursor-help text-end">
                        <span className="block">{column.label}</span>{" "}
                        <span className="block text-xs font-normal">
                          aggregate
                        </span>
                      </button>
                    }
                  />
                  <TooltipContent side="top" className="max-w-56">
                    {column.hint}
                  </TooltipContent>
                </Tooltip>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {keys.flatMap((key) => {
            const result = byKey.get(key);
            if (!result) return [];

            const values = people.map((person) =>
              finiteValue(result, person.entityId)
            );
            const cells = compareGaps(values, result.direction);
            const reference = compareReference(values);

            return [
              <tr key={key} className="hover:bg-muted/50">
                <th
                  scope="row"
                  className={cn(
                    STICKY_START,
                    "border-b px-3 py-1.5 text-start font-normal"
                  )}
                >
                  <span className="inline-flex items-center">
                    <MetricHelpTooltip
                      label={result.label}
                      help={helpByKey.get(key) ?? metricHelp(result)}
                    >
                      <span className="cursor-help">
                        {result.label}
                        <UnitNote result={result} className="ms-1.5 text-xs" />
                      </span>
                    </MetricHelpTooltip>
                    <DirectionMark direction={result.direction} />
                  </span>
                </th>
                {people.map((person, index) => {
                  const cell = cells[index]!;
                  return (
                    <td
                      key={person.entityId}
                      className={cn(
                        "border-b px-3 py-1.5 text-end tabular-nums",
                        cell.best && "bg-success/15 font-semibold text-success"
                      )}
                    >
                      <MetricValue
                        result={result}
                        entityId={person.entityId}
                        personName={person.displayName}
                      />
                      {cell.best ? (
                        <span className="sr-only"> (best)</span>
                      ) : null}
                      <span className={GAP_SLOT}>{formatGap(cell.gap)}</span>
                    </td>
                  );
                })}
                {REFERENCE_COLUMNS.map((column) => (
                  <td
                    key={column.key}
                    className={cn(
                      "border-b bg-muted px-3 py-1.5 text-end text-muted-foreground tabular-nums",
                      column.key === "median" && "border-s"
                    )}
                  >
                    {formatMetricFigure(
                      reference[column.key],
                      result.format,
                      result.unit
                    )}
                  </td>
                ))}
              </tr>,
            ];
          })}
        </tbody>
      </table>
    </div>
  );
}

function DirectionMark({ direction }: { direction: MetricDirection }) {
  const label = directionLabel(direction);
  if (!label) return null;

  const Icon = direction === "higher_is_better" ? ArrowUp : ArrowDown;

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type="button"
            aria-label={label}
            className="ms-1 inline-flex size-5 shrink-0 items-center justify-center rounded-sm text-muted-foreground hover:text-foreground"
          >
            <Icon className="size-3.5" aria-hidden />
          </button>
        }
      />
      <TooltipContent side="top">{label}</TooltipContent>
    </Tooltip>
  );
}
