import { Link } from "@tanstack/react-router";

import {
  MetricHelpTooltip,
  MetricValue,
} from "@/components/portal/metrics-table";
import type { MembersGridMember } from "@/components/widgets/dashboard/members-grid";
import { metricHelp, type MetricHelpText } from "@/lib/insight/metric-help";
import {
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import { compareGaps } from "@/lib/portal/metrics-view";
import { cn } from "@/lib/utils";

const STICKY_START = "sticky start-0 z-10 bg-card";

export function MetricsCompareTable({
  people,
  keys,
  byKey,
  helpByKey,
}: {
  people: readonly MembersGridMember[];
  keys: readonly string[];
  byKey: ReadonlyMap<string, NormalizedMetricResult>;
  helpByKey: ReadonlyMap<string, MetricHelpText>;
}) {
  return (
    <div className="max-h-[75svh] overflow-auto">
      <table className="w-full border-separate border-spacing-0 text-sm">
        <caption className="sr-only">Metrics compared side by side</caption>
        <thead>
          <tr>
            <th
              scope="col"
              className={cn(STICKY_START, "top-0 z-20 border-b px-3 py-2 text-start text-xs font-medium tracking-wider text-muted-foreground uppercase")}
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
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {keys.flatMap((key) => {
            const result = byKey.get(key);
            if (!result) return [];

            const { leader, gaps } = compareGaps(
              people.map((person) => {
                const { value } = forEntity(result, person.entityId);
                return value != null && Number.isFinite(value) ? value : null;
              }),
            );

            return [
              <tr key={key} className="hover:bg-muted/50">
                <th
                  scope="row"
                  className={cn(STICKY_START, "border-b px-3 py-1.5 text-start font-normal")}
                >
                  <MetricHelpTooltip
                    label={result.label}
                    help={helpByKey.get(key) ?? metricHelp(result)}
                  >
                    <span className="cursor-help">{result.label}</span>
                  </MetricHelpTooltip>
                </th>
                {people.map((person, index) => {
                  const gap = gaps[index];
                  return (
                    <td
                      key={person.entityId}
                      className={cn(
                        "border-b px-3 py-1.5 text-end tabular-nums",
                        index === leader && "font-semibold",
                      )}
                    >
                      <MetricValue
                        result={result}
                        entityId={person.entityId}
                        personName={person.displayName}
                      />
                      {index === leader ? <span className="sr-only"> (highest)</span> : null}
                      <span className="ms-2 inline-block w-14 text-start text-xs font-normal text-muted-foreground">
                        {gap == null ? "" : gap === 0 ? "≈" : `−${Math.abs(gap)}%`}
                      </span>
                    </td>
                  );
                })}
              </tr>,
            ];
          })}
        </tbody>
      </table>
    </div>
  );
}
