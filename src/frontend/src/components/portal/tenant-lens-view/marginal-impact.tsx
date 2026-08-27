import { Card, CardContent } from "@/components/ui/card";
import { fmtCompact } from "@/lib/portal/metric-stats";
import type { TenantSectionSpec } from "@/lib/portal/lens-configs";

import { marginalImpact } from "./derived";
import { sectionNeeds, type ResolveView } from "./plan";
import { tenantData } from "./data";

/**
 * What fixing the worst pipelines would buy: the gate pass rate re-computed
 * with the top-N failing pipelines' failures counted as passes. Derived from
 * ci.runs breakdown rows with the gate constants in `derived.ts` — the same
 * definition silver uses.
 */
export function MarginalImpactSection({
  section,
  resolve,
}: {
  section: Extract<TenantSectionSpec, { kind: "marginal-impact" }>;
  resolve: ResolveView;
}) {
  const r = resolve(sectionNeeds(section, "day")[0]);
  if (!r) return null;
  const impact = marginalImpact(tenantData(r).breakdown);
  if (!impact || impact.steps.length === 0) return null;

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
          <p className="text-sm">
            Today: <span className="font-semibold tabular-nums">{impact.currentRate.toFixed(1)}%</span>{" "}
            <span className="text-xs text-muted-foreground">
              over {fmtCompact(impact.gateRuns)} gate runs
            </span>
          </p>
          <ol className="flex flex-col gap-1 text-xs">
            {impact.steps.map((step) => (
              <li key={step.n} className="flex items-baseline justify-between gap-2">
                <span className="truncate text-muted-foreground" title={step.pipelines.join(", ")}>
                  Fix {step.n === 1 ? "the worst pipeline" : `the worst ${step.n}`}:{" "}
                  {step.pipelines.join(", ")}
                </span>
                <span className="whitespace-nowrap tabular-nums">
                  {step.rate.toFixed(1)}%{" "}
                  <span className="text-[var(--success)]">
                    (+{step.delta.toFixed(1)} pts)
                  </span>
                </span>
              </li>
            ))}
          </ol>
        </CardContent>
      </Card>
    </section>
  );
}
