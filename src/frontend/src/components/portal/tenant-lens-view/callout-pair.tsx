import { Card, CardContent } from "@/components/ui/card";
import { formatMetricValue } from "@/lib/format";
import { TEXT_FIGURE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import type { TenantSectionSpec } from "@/lib/portal/lens-configs";

import { calloutPair } from "./derived";
import { sectionNeeds, type ResolveView } from "./plan";
import { tenantData } from "./data";

/**
 * The org headline next to the unweighted mean over a dimension: the
 * headline weighs every run, the mean weighs every group once, and the gap
 * between them is how much the busiest groups dominate the story.
 */
export function CalloutPairSection({
  section,
  resolve,
}: {
  section: Extract<TenantSectionSpec, { kind: "callout-pair" }>;
  resolve: ResolveView;
}) {
  const needs = sectionNeeds(section, "day");
  const r = resolve(needs[0]);
  const breakdown = resolve(needs[1]);
  if (!r || !breakdown) return null;
  const pair = calloutPair(
    tenantData(r).value,
    tenantData(breakdown).breakdown,
    section.dimension
  );
  if (!pair) return null;

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {section.title}
      </p>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(14rem,1fr))] gap-3">
        <Card>
          <CardContent className="p-4">
            <p className="text-xs font-medium text-muted-foreground">
              Org-wide (every run counts)
            </p>
            <div className={cn("mt-1", TEXT_FIGURE)}>
              {formatMetricValue(pair.headline, r.format, r.unit)}
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="p-4">
            <p className="text-xs font-medium text-muted-foreground">
              Typical {section.dimension} (each counts once)
            </p>
            <div className={cn("mt-1", TEXT_FIGURE)}>
              {formatMetricValue(pair.unweightedMean, r.format, r.unit)}
            </div>
            <p className="text-xs text-muted-foreground">
              unweighted mean over {pair.groups} groups
            </p>
          </CardContent>
        </Card>
      </div>
    </section>
  );
}
