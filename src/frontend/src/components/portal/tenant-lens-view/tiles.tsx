import { Delta } from "@/components/portal/domain-lens-view";
import { MetricName } from "@/components/widgets/metric-help-tooltip";
import { Card, CardContent } from "@/components/ui/card";
import { formatMetricValue } from "@/lib/format";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import { TEXT_FIGURE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

import { tenantData } from "./data";

/** Headline and stat tiles: one org-wide value with its previous-period delta. */
export function TileRow({
  metrics,
  data,
  previous,
  minWidth,
}: {
  metrics: readonly string[];
  data: Map<string, NormalizedMetricResult>;
  previous: Map<string, NormalizedMetricResult> | null;
  minWidth: string;
}) {
  const tiles = metrics
    .map((key) => {
      const r = data.get(key);
      if (!r) return null;
      const now = tenantData(r).value;
      if (now == null) return null;
      const prevResult = previous?.get(key);
      const prev = prevResult ? tenantData(prevResult).value : null;
      return { key, r, now, prev };
    })
    .filter((tile): tile is NonNullable<typeof tile> => tile != null);
  if (!tiles.length) return null;

  return (
    <div
      className="grid gap-3"
      style={{
        gridTemplateColumns: `repeat(auto-fit, minmax(${minWidth}, 1fr))`,
      }}
    >
      {tiles.map((tile) => (
        <Card key={tile.key}>
          <CardContent className="p-4">
            <div className="flex items-center justify-between gap-2">
              <MetricName
                metric={tile.r}
                className="text-xs font-medium text-muted-foreground"
              />
              <Delta
                now={tile.now}
                prev={tile.prev}
                direction={tile.r.direction}
              />
            </div>
            <div className={cn("mt-1", TEXT_FIGURE)}>
              {formatMetricValue(tile.now, tile.r.format, tile.r.unit)}
            </div>
            <div className="text-xs text-muted-foreground">org-wide</div>
          </CardContent>
        </Card>
      ))}
    </div>
  );
}
