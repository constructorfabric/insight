import { Link } from "@tanstack/react-router";

import {
  MetricHelpTooltip,
  MetricValue,
  UnitNote,
} from "@/components/portal/metrics-table";
import type { MembersGridMember } from "@/components/widgets/dashboard/members-grid";
import { metricHelp, type MetricHelpText } from "@/lib/insight/metric-help";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import {
  compareGaps,
  finiteValue,
  formatGap,
  formatGapDetail,
} from "@/lib/portal/metrics-view";
import { cn } from "@/lib/utils";

const STICKY_EDGE = "sticky start-0 bg-card";
const GAP_SLOT =
  "ms-2 inline-block shrink-0 overflow-hidden text-start align-bottom text-xs font-normal whitespace-nowrap text-ellipsis text-muted-foreground";

// INVARIANT: 8ch fits "−100%"; one extra ch keeps a longer label from clipping.
function gapSlotWidth(labels: readonly string[]): string {
  const longest = labels.reduce(
    (width, label) => Math.max(width, label.length),
    0
  );
  return `${Math.max(8, longest + 1)}ch`;
}

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
  const rows = keys.flatMap((key) => {
    const result = byKey.get(key);
    if (!result) return [];

    return [
      {
        key,
        result,
        cells: compareGaps(
          people.map((person) => finiteValue(result, person.entityId)),
          result.direction
        ),
      },
    ];
  });

  const slots = people.map((_, index) =>
    gapSlotWidth(rows.map((row) => formatGap(row.cells[index]!.gap)))
  );

  return (
    <div className="max-h-[75svh] overflow-auto">
      <table className="w-full border-separate border-spacing-0 text-sm">
        <caption className="sr-only">Metrics compared side by side</caption>
        <thead>
          <tr>
            <th
              scope="col"
              className={cn(
                STICKY_EDGE,
                "top-0 z-30 min-w-44 border-e border-b px-3 py-2 text-start text-xs font-medium tracking-wider text-muted-foreground uppercase"
              )}
            >
              Metric
            </th>
            {people.map((person, index) => {
              const role = roles.get(person.entityId);
              const slot = slots[index]!;
              return (
                <th
                  key={person.entityId}
                  scope="col"
                  className="sticky top-0 z-20 min-w-48 border-b bg-card px-3 py-2 text-end font-medium"
                >
                  <div className="flex w-0 min-w-full items-baseline justify-end">
                    <Link
                      to="/ic/$person/personal"
                      params={{ person: person.entityId }}
                      title={person.displayName}
                      className="min-w-0 truncate hover:underline"
                    >
                      {person.displayName}
                    </Link>
                    <span
                      aria-hidden
                      className={GAP_SLOT}
                      style={{ width: slot }}
                    />
                  </div>
                  {role ? (
                    <div
                      title={role}
                      className="w-0 min-w-full truncate text-xs font-normal text-muted-foreground"
                      style={{ paddingInlineEnd: `calc(${slot} + 0.5rem)` }}
                    >
                      {role}
                    </div>
                  ) : null}
                </th>
              );
            })}
          </tr>
        </thead>
        <tbody>
          {rows.map(({ key, result, cells }) => (
            <tr key={key} className="hover:bg-muted/50">
              <th
                scope="row"
                className={cn(
                  STICKY_EDGE,
                  "z-10 min-w-44 border-e border-b px-3 py-1.5 text-start font-normal"
                )}
              >
                <div className="max-w-52">
                  <MetricHelpTooltip
                    label={result.label}
                    help={helpByKey.get(key) ?? metricHelp(result)}
                  >
                    <span className="cursor-help">
                      {result.label}
                      <UnitNote result={result} className="ms-1.5 text-xs" />
                    </span>
                  </MetricHelpTooltip>
                </div>
              </th>
              {people.map((person, index) => {
                const cell = cells[index]!;
                const gap = formatGap(cell.gap);
                const gapDetail = formatGapDetail(cell.gap);
                const slot = slots[index]!;
                return (
                  <td
                    key={person.entityId}
                    className={cn(
                      "min-w-48 border-b px-3 py-1.5 text-end whitespace-nowrap tabular-nums",
                      cell.best && "font-semibold"
                    )}
                  >
                    <MetricValue
                      compact
                      result={result}
                      entityId={person.entityId}
                      personName={person.displayName}
                    />
                    {cell.best ? (
                      <span className="sr-only"> (best)</span>
                    ) : null}
                    <span
                      className={GAP_SLOT}
                      style={{ width: slot }}
                      title={gapDetail ?? undefined}
                    >
                      {gap}
                    </span>
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
