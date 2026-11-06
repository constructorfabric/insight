import { Link } from "@tanstack/react-router";

import type { MembersGridMember } from "@/components/widgets/dashboard/members-grid";
import { formatMetricValue } from "@/lib/format";
import type { CompareRow } from "@/lib/portal/metrics-view";
import { cn } from "@/lib/utils";

const STICKY_START = "sticky start-0 z-10 bg-card";

export function MetricsCompareTable({
  people,
  rows,
}: {
  people: readonly MembersGridMember[];
  rows: readonly CompareRow[];
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
          {rows.map((row) => (
            <tr key={row.key} className="hover:bg-muted/50">
              <th
                scope="row"
                className={cn(STICKY_START, "border-b px-3 py-1.5 text-start font-normal")}
              >
                {row.label}
              </th>
              {row.values.map((value, index) => (
                <td
                  key={people[index]?.entityId ?? index}
                  className="border-b px-3 py-1.5 text-end tabular-nums"
                >
                  {value == null ? "—" : formatMetricValue(value, row.format, row.unit)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
