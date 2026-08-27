import { Card, CardContent } from "@/components/ui/card";
import { cn } from "@/lib/utils";
import type { TenantSectionSpec } from "@/lib/portal/lens-configs";

import { weeklyVerdicts, type StabilityVerdict } from "./derived";
import { sectionNeeds, type ResolveView } from "./plan";
import { tenantData } from "./data";

const DEFAULT_MIN_WEEKS = 5;

const VERDICT_STYLE: Record<StabilityVerdict, { label: string; className: string }> = {
  solid: { label: "solid", className: "text-[var(--success)]" },
  healthy: { label: "healthy", className: "text-[var(--success)]" },
  watch: { label: "watch", className: "text-muted-foreground" },
  erratic: { label: "erratic", className: "text-[var(--destructive)]" },
  struggling: { label: "struggling", className: "text-[var(--destructive)]" },
};

/** Mean weekly rate and its volatility per value, resolved to a verdict. */
export function VerdictTableSection({
  section,
  resolve,
}: {
  section: Extract<TenantSectionSpec, { kind: "verdict-table" }>;
  resolve: ResolveView;
}) {
  // The need is week-bucketed by construction (`sectionNeeds`).
  const r = resolve(sectionNeeds(section, "week")[0]);
  if (!r) return null;
  const { verdicts, thin } = weeklyVerdicts(
    tenantData(r).series,
    section.dimension,
    section.minWeeks ?? DEFAULT_MIN_WEEKS
  );
  if (verdicts.length === 0) return null;

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {section.title}
      </p>
      <Card>
        <CardContent className="p-4">
          {section.description ? (
            <p className="mb-3 text-xs text-muted-foreground">
              {section.description}
            </p>
          ) : null}
          <table className="w-full text-xs">
            <thead>
              <tr className="text-left text-muted-foreground">
                <th className="pb-2 font-medium">{section.dimension}</th>
                <th className="pb-2 text-right font-medium">weekly mean</th>
                <th className="pb-2 text-right font-medium">spread (σ)</th>
                <th className="pb-2 text-right font-medium">weeks</th>
                <th className="pb-2 text-right font-medium">verdict</th>
              </tr>
            </thead>
            <tbody>
              {verdicts.map((row) => (
                <tr key={row.value} className="border-t border-border">
                  <td className="max-w-48 truncate py-1.5" title={row.label}>
                    {row.label}
                  </td>
                  <td className="py-1.5 text-right tabular-nums">
                    {row.mean.toFixed(1)}%
                  </td>
                  <td className="py-1.5 text-right tabular-nums">
                    {row.stddev.toFixed(1)}
                  </td>
                  <td className="py-1.5 text-right tabular-nums">{row.weeks}</td>
                  <td
                    className={cn(
                      "py-1.5 text-right font-medium",
                      VERDICT_STYLE[row.verdict].className
                    )}
                  >
                    {VERDICT_STYLE[row.verdict].label}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {thin > 0 ? (
            <p className="mt-2 text-xs text-muted-foreground">
              {thin} with under {section.minWeeks ?? DEFAULT_MIN_WEEKS} weeks of
              history stay unjudged.
            </p>
          ) : null}
        </CardContent>
      </Card>
    </section>
  );
}
