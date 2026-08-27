import { Card, CardContent } from "@/components/ui/card";
import { formatMetricValue } from "@/lib/format";
import type { TenantSectionSpec } from "@/lib/portal/lens-configs";

import { dumbbellPairs } from "./derived";
import { sectionNeeds, type ResolveView } from "./plan";
import { tenantData } from "./data";

const DUMBBELL_LIMIT = 12;

/** One metric split two ways per dimension value — e.g. fail vs pass medians. */
export function DumbbellSection({
  section,
  resolve,
}: {
  section: Extract<TenantSectionSpec, { kind: "dumbbell" }>;
  resolve: ResolveView;
}) {
  const r = resolve(sectionNeeds(section, "day")[0]);
  if (!r) return null;
  const rows = dumbbellPairs(
    tenantData(r).breakdown,
    section.dimension,
    section.splitBy,
    section.left,
    section.right
  ).slice(0, DUMBBELL_LIMIT);
  if (rows.length < 2) return null;
  const max = Math.max(...rows.flatMap((row) => [row.left, row.right]), 1e-9);
  const position = (value: number) => `${(value / max) * 100}%`;

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {section.title}
      </p>
      <Card>
        <CardContent className="flex flex-col gap-2 p-4">
          {section.description ? (
            <p className="text-xs text-muted-foreground">{section.description}</p>
          ) : null}
          <p className="flex gap-4 text-xs text-muted-foreground">
            <span>
              <span className="mr-1 inline-block size-2 rounded-full bg-[var(--destructive)] align-middle" />
              {section.left}
            </span>
            <span>
              <span className="mr-1 inline-block size-2 rounded-full bg-[var(--success)] align-middle" />
              {section.right}
            </span>
          </p>
          {rows.map((row) => (
            <div key={row.value} className="flex items-center gap-2 text-xs">
              <span
                className="w-40 shrink-0 truncate text-muted-foreground"
                title={row.label}
              >
                {row.label}
              </span>
              <div className="relative h-3 flex-1">
                <div
                  className="absolute top-1/2 h-px -translate-y-1/2 bg-border"
                  style={{
                    left: position(Math.min(row.left, row.right)),
                    width: `calc(${position(Math.abs(row.left - row.right))})`,
                  }}
                />
                <span
                  className="absolute top-1/2 size-2 -translate-x-1/2 -translate-y-1/2 rounded-full bg-[var(--destructive)]"
                  style={{ left: position(row.left) }}
                />
                <span
                  className="absolute top-1/2 size-2 -translate-x-1/2 -translate-y-1/2 rounded-full bg-[var(--success)]"
                  style={{ left: position(row.right) }}
                />
              </div>
              <span className="w-28 shrink-0 text-right tabular-nums">
                {formatMetricValue(row.left, r.format, r.unit)} /{" "}
                {formatMetricValue(row.right, r.format, r.unit)}
              </span>
            </div>
          ))}
        </CardContent>
      </Card>
    </section>
  );
}
