import { ArrowDown } from "lucide-react";

import { formatMetricValue } from "@/lib/format";
import type { MetricFormat } from "@/api/metric-results-client";
import {
  PEER_FILL,
  PEER_TEXT,
  type PeerStats,
  type PeerStatusWithNeutral,
} from "@/lib/peers";
import { STATUS_SURFACE_CLASS } from "@/lib/status";
import { cn } from "@/lib/utils";

export interface PeerComparisonProps {
  value: number;
  stats: PeerStats;
  status: PeerStatusWithNeutral;
  higherIsBetter: boolean;
  format: MetricFormat;
  unit: string | null;
}

/**
 * Quartile strip: cohort zones (bottom / interquartile pack / top, colored by
 * direction), median tick, and the person's value marker. Binless — cohort
 * percentiles come from the peer view.
 */
export function PeerComparison({
  value,
  stats,
  status,
  higherIsBetter,
  format,
  unit,
}: PeerComparisonProps) {
  const span = Math.max(1e-9, stats.max - stats.min);
  const pct = (v: number) =>
    ((Math.max(stats.min, Math.min(stats.max, v)) - stats.min) / span) * 100;
  const p25Left = pct(stats.p25);
  const p50Left = pct(stats.p50);
  const p75Left = pct(stats.p75);
  const valueLeft = pct(value);
  const bottomZone = higherIsBetter
    ? STATUS_SURFACE_CLASS.bad
    : STATUS_SURFACE_CLASS.good;
  const topZone = higherIsBetter
    ? STATUS_SURFACE_CLASS.good
    : STATUS_SURFACE_CLASS.bad;

  return (
    <div className="mt-4">
      <div className="relative h-3.5 w-full select-none">
        <ArrowDown
          className={cn(
            "absolute -top-4 size-3 -translate-x-1/2",
            PEER_TEXT[status],
          )}
          style={{ left: `${valueLeft}%` }}
          strokeWidth={3}
          aria-hidden
        />
        <div className="absolute inset-x-0 top-1/2 h-2 -translate-y-1/2 overflow-hidden rounded-sm">
          <div
            className={cn("absolute inset-y-0 left-0", bottomZone)}
            style={{ width: `${p25Left}%` }}
          />
          <div
            className="absolute inset-y-0 bg-muted"
            style={{ left: `${p25Left}%`, width: `${p75Left - p25Left}%` }}
          />
          <div
            className={cn("absolute inset-y-0", topZone)}
            style={{ left: `${p75Left}%`, right: 0 }}
          />
        </div>
        <div
          className="absolute inset-y-0 w-px bg-foreground/60"
          style={{ left: `${p50Left}%` }}
          aria-hidden
        />
        <div
          className={cn(
            "absolute inset-y-0 w-[3px] -translate-x-1/2 rounded-sm ring-2 ring-background",
            PEER_FILL[status],
          )}
          style={{ left: `${valueLeft}%` }}
        />
      </div>
      {/* Min/max hug the bar (mt-1); the value pointer sits above the bar
          (ArrowDown), clear of these labels. */}
      <div className="mt-1 grid grid-cols-2 gap-3 text-[10px] tabular-nums">
        <span className="text-left text-muted-foreground">
          {formatMetricValue(stats.min, format, unit)}
        </span>
        <span className="text-right text-muted-foreground">
          {formatMetricValue(stats.max, format, unit)}
        </span>
      </div>
    </div>
  );
}
