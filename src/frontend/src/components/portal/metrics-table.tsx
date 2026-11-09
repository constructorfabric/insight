import type { ReactElement } from "react";
import { Link } from "@tanstack/react-router";
import { ArrowDown, ArrowUp, ArrowUpRight, ChevronRight } from "lucide-react";

import { evidenceSelection } from "@/api/metric-drilldown-client";
import { useMetricEvidenceOptional } from "@/components/metric-evidence-context";
import { Tooltip, TooltipContent, TooltipTrigger } from "@gears-frontx/ui-kit";
import { metricHelp, type MetricHelpText } from "@/lib/insight/metric-help";
import { formatRange } from "@/lib/date-range-label";
import { formatMetricFigure, formatMetricValue } from "@/lib/format";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import {
  finiteValue,
  headerUnit,
  type MetricGroup,
  type MetricsRow,
  type SortDirection,
} from "@/lib/portal/metrics-view";
import { cn } from "@/lib/utils";

export interface MetricsSort {
  key: string;
  direction: SortDirection;
}

const GROUP_ROW_HEIGHT = "h-7";
const BELOW_GROUP_ROW = "top-7";
const HEADER_CELL = "sticky z-20 border-b bg-card px-2 text-start";
const NAME_CELL = "sticky start-0 z-10 bg-card";

export function MetricsTable({
  rows,
  groups,
  byKey,
  helpByKey,
  roles,
  sort,
  onSort,
  onToggle,
  onScope,
}: {
  rows: readonly MetricsRow[];
  groups: readonly MetricGroup[];
  byKey: ReadonlyMap<string, NormalizedMetricResult>;
  helpByKey: ReadonlyMap<string, MetricHelpText>;
  roles: ReadonlyMap<string, string>;
  sort: MetricsSort | null;
  onSort: (metricKey: string) => void;
  onToggle: (entityId: string) => void;
  onScope: (entityId: string) => void;
}) {
  const keys = groups.flatMap((group) => group.keys);
  const groupStarts = new Set(groups.slice(1).map((group) => group.keys[0]));

  return (
    <div className="max-h-[calc(100svh-13rem)] overflow-auto rounded-lg border bg-card">
      <table className="w-full border-separate border-spacing-0 text-sm">
        <caption className="sr-only">People × metrics</caption>
        <thead>
          <tr className={GROUP_ROW_HEIGHT}>
            <th
              scope="col"
              rowSpan={2}
              className={cn(
                HEADER_CELL,
                NAME_CELL,
                "top-0 z-30 min-w-64 border-e align-bottom py-2 font-semibold",
              )}
            >
              Person
            </th>
            {groups.map((group) => (
              <th
                key={group.family}
                scope="colgroup"
                colSpan={group.keys.length}
                className={cn(
                  HEADER_CELL,
                  "top-0 border-s text-xs font-medium tracking-wide whitespace-nowrap text-muted-foreground uppercase",
                )}
              >
                {group.name}
              </th>
            ))}
          </tr>
          <tr>
            {keys.map((key) => {
              const result = byKey.get(key);
              const label = result?.label ?? key;
              const help = helpByKey.get(key) ?? (result ? metricHelp(result) : null);
              const direction = sort?.key === key ? sort.direction : undefined;
              return (
                <th
                  key={key}
                  scope="col"
                  aria-sort={direction}
                  className={cn(
                    HEADER_CELL,
                    BELOW_GROUP_ROW,
                    "min-w-28 max-w-36 py-2 align-bottom",
                    groupStarts.has(key) && "border-s",
                  )}
                >
                  <MetricHelpTooltip label={label} help={help}>
                    <button
                      type="button"
                      aria-label={`Sort by ${label}`}
                      onClick={() => onSort(key)}
                      className="flex w-full cursor-pointer items-end justify-end gap-1 text-end text-xs leading-tight font-semibold hover:text-primary"
                    >
                      <span className="whitespace-normal">
                        {label}
                        <UnitNote result={result} className="block" />
                      </span>
                      {direction === "descending" ? (
                        <ArrowDown className="size-3 shrink-0" aria-hidden />
                      ) : direction === "ascending" ? (
                        <ArrowUp className="size-3 shrink-0" aria-hidden />
                      ) : null}
                    </button>
                  </MetricHelpTooltip>
                </th>
              );
            })}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.entityId} className="group hover:bg-muted/60">
              <th
                scope="row"
                className={cn(
                  NAME_CELL,
                  "border-e border-b px-2 py-1 text-start font-normal whitespace-nowrap group-hover:bg-muted",
                )}
              >
                <div className="flex items-center gap-1">
                  <span
                    aria-hidden
                    className="shrink-0"
                    style={{ inlineSize: `${row.depth * 1.25}rem` }}
                  />
                  {row.hasReports ? (
                    <button
                      type="button"
                      aria-label={`${row.expanded ? "Collapse" : "Expand"} ${row.displayName}`}
                      aria-expanded={row.expanded}
                      onClick={() => onToggle(row.entityId)}
                      className="flex size-5 shrink-0 cursor-pointer items-center justify-center rounded text-muted-foreground hover:bg-muted hover:text-foreground"
                    >
                      <ChevronRight
                        className={cn("size-3.5 transition-transform", row.expanded && "rotate-90")}
                      />
                    </button>
                  ) : (
                    <span aria-hidden className="size-5 shrink-0" />
                  )}
                  <button
                    type="button"
                    onClick={() => onScope(row.entityId)}
                    className="cursor-pointer truncate font-medium hover:underline"
                  >
                    {row.displayName}
                  </button>
                  {roles.get(row.entityId) ? (
                    <span className="truncate text-xs text-muted-foreground">
                      {roles.get(row.entityId)}
                    </span>
                  ) : null}
                  <Link
                    to="/ic/$person/personal"
                    params={{ person: row.entityId }}
                    aria-label={`Open ${row.displayName}`}
                    className="ms-auto shrink-0 text-muted-foreground opacity-0 group-hover:opacity-100 hover:text-foreground focus-visible:opacity-100"
                  >
                    <ArrowUpRight className="size-3.5" />
                  </Link>
                </div>
              </th>
              {keys.map((key) => (
                <td
                  key={key}
                  className={cn(
                    "border-b px-2 py-1 text-end whitespace-nowrap tabular-nums",
                    groupStarts.has(key) && "border-s",
                  )}
                >
                  <MetricValue
                    result={byKey.get(key)}
                    entityId={row.entityId}
                    personName={row.displayName}
                  />
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function MetricHelpTooltip({
  label,
  help,
  children,
}: {
  label: string;
  help: MetricHelpText | null;
  children: ReactElement;
}) {
  return (
    <Tooltip>
      <TooltipTrigger render={children} />
      <TooltipContent side="top" className="max-w-72">
        <span className="flex flex-col gap-1 leading-snug">
          <span className="font-medium">{label}</span>
          {help?.description ? <span>{help.description}</span> : null}
          {help?.explanation ? <span className="opacity-80">{help.explanation}</span> : null}
        </span>
      </TooltipContent>
    </Tooltip>
  );
}

export function MetricValue({
  result,
  entityId,
  personName,
}: {
  result: NormalizedMetricResult | undefined;
  entityId: string;
  personName: string;
}) {
  const evidence = useMetricEvidenceOptional();
  const value = result ? finiteValue(result, entityId) : null;
  if (!result || value == null) {
    return <span className="text-muted-foreground">—</span>;
  }

  const text = formatMetricFigure(value, result.format, result.unit);
  const fullText = formatMetricValue(value, result.format, result.unit);
  const selection = result.drilldown
    ? evidenceSelection(result.selection, entityId)
    : null;
  if (!selection || !evidence) return <span>{text}</span>;

  return (
    <button
      type="button"
      aria-label={`${personName} — ${result.label}: ${fullText}`}
      onClick={() =>
        evidence.openEvidenceTargets([{ selection, label: result.label }], {
          activeMetricKey: selection.metric_key,
          title: [
            result.label,
            personName,
            fullText,
            formatRange(selection.period.from, selection.period.to),
          ].join(" · "),
        })
      }
      className="cursor-pointer hover:text-primary hover:underline"
    >
      {text}
    </button>
  );
}

export function UnitNote({
  result,
  className,
}: {
  result: NormalizedMetricResult | undefined;
  className: string;
}) {
  const unit = result ? headerUnit(result.label, result.format, result.unit) : null;
  if (!unit) return null;
  return <span className={cn("font-normal text-muted-foreground", className)}>{unit}</span>;
}
