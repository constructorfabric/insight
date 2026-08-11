import { useState } from "react";
import { MetricName } from "@/components/widgets/metric-help-tooltip";
import { ChevronDown, ChevronRight } from "lucide-react";

import { Card, CardContent } from "@/components/ui/card";
import {
  dimensionColorSeed,
  dimensionLabel,
  dimensionSeriesKey,
} from "@/components/widgets/metric-views/dimension-series";
import { MetricSublabel } from "@/components/widgets/dashboard/metric-sublabel";
import { MetricCardActions } from "@/components/widgets/metric-views/metric-card-actions";
import {
  formatMetricNumber,
  formatMetricValue,
  metricDisplayUnit,
} from "@/lib/format";
import {
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import { seriesColors } from "@/lib/series-colors";
import { evidenceSelection } from "@/api/metric-drilldown-client";
import { TEXT_FIGURE } from "@/lib/type-scale";

export interface MetricSummaryCardProps {
  metric: NormalizedMetricResult;
  entityId: string;
}

/**
 * Modality headline card: period total, plus a collapsible
 * proportional breakdown over the metric's dimension groups (ribbon +
 * legend). The breakdown section renders only when at least two groups have
 * data — a single-source metric reads as a plain summary card.
 *
 * No standing and no colour. The card says how much of a modality there was
 * and what it was made of; whether that is good is a judgment the section
 * around it deliberately does not make, and a coloured card makes it anyway
 * — louder than any sentence next to it.
 */
export function MetricSummaryCard({
  metric,
  entityId,
}: MetricSummaryCardProps) {
  const [open, setOpen] = useState(false);
  const evidence = metric.drilldown
    ? evidenceSelection(
        metric.selection,
        entityId,
        undefined,
        undefined,
        metric.breakdown?.dimensions
      )
    : null;

  const data = forEntity(metric, entityId);
  const value = data.value;
  // Eligibility (observed / suppressed / flat pool / neutral direction) and
  // the quartile rank come from the shared standing derivation — same
  // verdict as the KPI tiles and the peer story by construction: red means
  // bottom quartile, in-pack is normal and stays uncolored.

  const rows = data.breakdown
    .filter((row) => (row.value ?? 0) > 0)
    .map((row) => ({
      key: dimensionSeriesKey(row.dimensions),
      colorSeed: dimensionColorSeed(row.dimensions),
      label: dimensionLabel(row.dimensions),
      value: row.value ?? 0,
    }))
    .sort((a, b) => b.value - a.value);
  const colorsBySeed = seriesColors(rows.map((row) => row.colorSeed));
  const rowsTotal = rows.reduce((sum, row) => sum + row.value, 0) || 1;
  const breakdownLabel = `By ${(metric.breakdown?.dimensions ?? []).join(" / ")}`;

  const displayUnit = metricDisplayUnit(metric.format, metric.unit);

  return (
    <Card className="relative h-full">
      <MetricCardActions evidence={evidence} label={metric.label} />
      <CardContent className="flex h-full flex-col gap-3">
        {/* KPI-tile line structure — label, sublabel slot, then the
            value on its own line — so narrow cards never truncate the label
            against the number, and all cards in a row share geometry (the
            sublabel reserves two lines whenever explanations are on). */}
        <div className="flex min-w-0 flex-col gap-1">
          <MetricName
            metric={metric}
            className="truncate pr-8 text-sm font-semibold"
          />
          <MetricSublabel
            description={metric.description}
            className="min-h-[2lh]"
          />
        </div>
        <span className="flex items-baseline gap-1 tabular-nums">
          <span className={TEXT_FIGURE}>
            {value == null
              ? "—"
              : metric.format === "percent"
                ? formatMetricValue(value, metric.format, metric.unit)
                : formatMetricNumber(value, metric.format)}
          </span>
          {value != null && displayUnit ? (
            <span className="text-sm text-muted-foreground">{displayUnit}</span>
          ) : null}
        </span>

        {rows.length > 1 ? (
          <>
            <button
              type="button"
              onClick={() => setOpen((v) => !v)}
              className="flex items-center gap-1.5 text-left text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
              aria-expanded={open}
            >
              {open ? (
                <ChevronDown className="size-3.5" />
              ) : (
                <ChevronRight className="size-3.5" />
              )}
              <span>{breakdownLabel}</span>
            </button>
            {open ? (
              <div className="flex flex-col gap-2">
                <div className="flex h-3 w-full overflow-hidden rounded-sm bg-muted">
                  {rows.map((row) => (
                    <span
                      key={row.key}
                      className="h-full min-w-[2px]"
                      style={{
                        width: `${(row.value / rowsTotal) * 100}%`,
                        backgroundColor: colorsBySeed[row.colorSeed],
                      }}
                      title={`${row.label}: ${formatMetricValue(row.value, metric.format, metric.unit)}`}
                    />
                  ))}
                </div>
                <ul className="flex flex-col gap-1 text-xs">
                  {rows.map((row) => (
                    <li
                      key={row.key}
                      className="flex items-center justify-between gap-2"
                    >
                      <span className="flex min-w-0 items-center gap-2">
                        <span
                          aria-hidden
                          className="size-2.5 shrink-0 rounded-[3px]"
                          style={{
                            backgroundColor: colorsBySeed[row.colorSeed],
                          }}
                        />
                        <span className="truncate">{row.label}</span>
                      </span>
                      <span className="shrink-0 text-muted-foreground tabular-nums">
                        {formatMetricValue(
                          row.value,
                          metric.format,
                          metric.unit
                        )}
                      </span>
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
          </>
        ) : null}
      </CardContent>
    </Card>
  );
}
