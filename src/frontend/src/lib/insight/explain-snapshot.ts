import type { MetricSnapshot } from "@/api/ai-client";
import type { KpiTileData } from "@/lib/insight/kpi-row";

export interface SnapshotContext {
  periodNoun: string;
  since: string;
  until: string;
  trend?: (number | null)[] | null;
}

/**
 * The tile, as the reader sees it, in the shape the explain endpoint takes.
 *
 * Everything here is already on screen. Sending the rendered strings rather
 * than raw numbers is deliberate: the explanation then describes the same
 * "41%" and "Team median 48%" the reader is looking at, instead of a rounding
 * of its own.
 */
export function metricSnapshot(
  tile: KpiTileData,
  { periodNoun, since, until, trend }: SnapshotContext
): MetricSnapshot {
  return {
    metric_key: tile.key,
    label: tile.label,
    value: tile.value,
    period: periodNoun,
    since,
    until,
    delta: tile.delta ? `${tile.delta.text} since last ${periodNoun}` : "",
    peer: peerLine(tile),
    help: tile.help?.description ?? "",
    trend: trend ?? [],
  };
}

function peerLine(tile: KpiTileData): string {
  if (!tile.medianLabel) return "";
  return tile.gapText
    ? `Team ${tile.medianLabel} · ${tile.gapText}`
    : `Team ${tile.medianLabel}`;
}
