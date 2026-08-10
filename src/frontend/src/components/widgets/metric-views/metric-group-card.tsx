import { ChevronRight } from "lucide-react";
import { MetricName } from "@/components/widgets/metric-help-tooltip";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Spinner } from "@/components/ui/spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { GroupCardEmpty } from "@/components/widgets/group-card-empty";
import { useSettings } from "@/hooks/use-settings";
import { formatMetricValue } from "@/lib/format";
import type { MetricGroup } from "@/lib/insight/groups";
import { peerStatusToStatus } from "@/lib/insight/peer-status";
import {
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import { formatGapMagnitude } from "@/lib/metrics/gap";
import {
  derivePeerStanding,
  type PeerStanding,
} from "@/lib/metrics/peer-standing";
import type { PeerStatusWithNeutral } from "@/lib/peers";
import {
  gradeSectionStanding,
  rankCounts,
  sectionStandingPhrase,
} from "@/lib/scoring";
import { STATUS_BG_CLASS, applyFocusStatus } from "@/lib/status";
import type { MetricCollectionResult } from "@/queries/metric-results";
import { cn } from "@/lib/utils";

export interface MetricGroupCardProps {
  def: MetricGroup;
  data: MetricCollectionResult;
  entityId: string;
  onOpen: () => void;
  onHover?: () => void;
  subtitle?: string;
  /**
   * Per-metric rank override (team view): replaces the single-entity
   * quartile scoring with a roster rollup computed by the caller.
   */
  rankByMetricKey?: Map<string, PeerStatusWithNeutral>;
}

interface CardRow {
  metric: NormalizedMetricResult;
  value: number | null;
  rank: PeerStatusWithNeutral;
  standing: PeerStanding;
}

const HEADLINE_TIER: Record<PeerStatusWithNeutral, number> = {
  bottom: 3,
  in_pack: 2,
  top: 1,
  neutral: 0,
};

export function MetricGroupCard({
  def,
  data,
  entityId,
  onOpen,
  onHover,
  subtitle,
  rankByMetricKey,
}: MetricGroupCardProps) {
  const { focusMode } = useSettings();

  if (data.isPending) {
    // Keep the card's identity while it loads: the name in the header, a
    // spinner in the body. Not interactive — nothing to open yet.
    return (
      <Card>
        <CardHeader>
          <CardTitle className="text-base font-semibold">{def.title}</CardTitle>
          {subtitle ? (
            <CardDescription className="text-xs text-muted-foreground">
              {subtitle}
            </CardDescription>
          ) : null}
        </CardHeader>
        <CardContent className="flex items-center justify-center py-6">
          <Spinner
            className="size-5 text-muted-foreground"
            aria-label={`Loading ${def.title}`}
          />
        </CardContent>
      </Card>
    );
  }
  if (data.isError) {
    return (
      <ComingSoon
        variant="card"
        state="error"
        label={def.title}
        onRetry={data.refetch}
      />
    );
  }

  const rows: CardRow[] = def.collection.metrics.flatMap((metricConfig) => {
    const metric = data.byKey.get(metricConfig.key);
    if (!metric) return [];
    const entityData = forEntity(metric, entityId);
    const peerRow =
      metric.peer?.values.find((v) => v.entity_id === entityId) ?? null;
    // All eligibility gates (observed / suppressed / flat pool / neutral
    // direction) live in the shared standing derivation, reused for the
    // headline's severity ordering and gap.
    const standing = derivePeerStanding(metric.direction, {
      value: entityData.value,
      peer: peerRow,
    });
    const rank = rankByMetricKey?.get(metric.metric_key) ?? standing.rank;
    return [{ metric, value: entityData.value, rank, standing }];
  });

  const counts = rankCounts(rows.map((row) => ({ row, rank: row.rank })));
  const status = applyFocusStatus(gradeSectionStanding(counts), focusMode);
  const badgeText = sectionStandingPhrase(counts);

  const headlineRow = pickHeadlineRow(rows);

  // Rows with a value, and only those. A "—" row spends a line saying nothing:
  // a null here is not a measured zero — that would print 0 — it is a metric no
  // connector feeds for this person, and the card's identity is not worth a
  // line of blank on every period they lack it. Same rule the headline row
  // follows.
  //
  // The lead goes FIRST in the list, not into a sentence above it. The card
  // used to state a headline chosen from every metric of the group while
  // listing three fixed keys, so it either named a metric the reader could not
  // see, or repeated one of them a line later. One list, lead at the top.
  const previewRows = def.card.preview
    .map((key) => rows.find((row) => row.metric.metric_key === key))
    .filter((row): row is CardRow => row != null && row.value != null);
  const preview = (
    headlineRow
      ? [headlineRow, ...previewRows.filter((row) => row !== headlineRow)]
      : previewRows
  ).slice(0, 4);
  const isEmpty = !rows.some((row) => row.value != null);

  return (
    <Card
      // An empty card has nothing to drill into: render a plain, unfocusable
      // div instead of a button, and drop the interactive affordances.
      render={
        isEmpty ? undefined : (
          <button
            type="button"
            onClick={onOpen}
            onMouseEnter={onHover}
            onFocus={onHover}
            aria-label={`Open ${def.title} details`}
          />
        )
      }
      className={cn(
        // Header→content on the card's own 12px rhythm (the preview stack's
        // gap-3), not the default 24px section gap.
        "gap-3",
        !isEmpty && "text-left transition-colors hover:bg-accent/50"
      )}
    >
      <CardHeader>
        <CardTitle className="flex items-center gap-1 text-base font-semibold">
          <span className="min-w-0 truncate">{def.title}</span>
          {/* Same standing affordance as the KPI tile — an empty card is not
              interactive and gets none. */}
          {isEmpty ? null : (
            <ChevronRight
              className="size-4 shrink-0 text-muted-foreground/60"
              aria-hidden
            />
          )}
        </CardTitle>
        {subtitle || !isEmpty ? (
          <CardDescription className="flex flex-col gap-1 text-xs">
            {subtitle ? (
              <span className="text-muted-foreground">{subtitle}</span>
            ) : null}
            {/* An empty card carries no standing — the badge would only
                restate the empty state below. */}
            {!isEmpty ? (
              <span className="flex items-center gap-1.5">
                <span
                  className={cn(
                    "size-1.5 shrink-0 rounded-full",
                    STATUS_BG_CLASS[status]
                  )}
                  aria-hidden
                />
                <span className="tabular-nums">{badgeText}</span>
              </span>
            ) : null}
          </CardDescription>
        ) : null}
      </CardHeader>
      <CardContent className="flex flex-1 flex-col gap-3">
        {isEmpty ? (
          <GroupCardEmpty />
        ) : (
          <>
            {preview.length > 0 ? (
              <ul className="flex flex-col gap-1.5">
                {preview.map((row) => {
                  const previewStatus = applyFocusStatus(
                    peerStatusToStatus(row.rank),
                    focusMode
                  );
                  // Only the lead carries its comparison: it is the reason the
                  // card is worth opening, and putting "vs median" on every row
                  // is how the old summary line and the rows ended up saying
                  // the same thing twice.
                  const gap = row === preview[0] ? gapPhrase(row) : null;
                  return (
                    <li
                      key={row.metric.metric_key}
                      className="flex flex-col gap-0.5 text-sm"
                    >
                      <span className="flex items-center gap-2">
                        <span
                          className={cn(
                            "size-2 shrink-0 rounded-full",
                            STATUS_BG_CLASS[previewStatus]
                          )}
                          aria-hidden
                        />
                        <MetricName
                          metric={row.metric}
                          className="min-w-0 flex-1 truncate text-muted-foreground"
                        />
                        {/* The dot carries the standing; the number stays a
                          number. Colouring both put two marks on one row, and
                          a card of five rows then read as five problems
                          instead of one section worth opening. */}
                        <span className="shrink-0 font-medium tabular-nums">
                          {row.value != null
                            ? formatMetricValue(
                                row.value,
                                row.metric.format,
                                row.metric.unit
                              )
                            : "—"}
                        </span>
                      </span>
                      {/* Under the row, not beside it: a card is a quarter of
                          the content width, and a third column there truncated
                          the names it was meant to explain. The indent lines
                          it up with the label. */}
                      {gap ? (
                        <span className="pl-4 text-xs text-muted-foreground tabular-nums">
                          {gap}
                        </span>
                      ) : null}
                    </li>
                  );
                })}
              </ul>
            ) : null}
          </>
        )}
      </CardContent>
    </Card>
  );
}

/**
 * The card's spotlight metric: the worst-ranked row, breaking ties by the
 * largest peer divergence so the number the header shows is the one most
 * worth explaining. Falls back to any row with a value when nothing ranks,
 * so a data-bearing card never headlines "No data".
 */
function pickHeadlineRow(rows: CardRow[]): CardRow | null {
  const ranked = rows
    .filter((row) => row.rank !== "neutral")
    .reduce<CardRow | null>((best, row) => {
      if (!best) return row;
      if (HEADLINE_TIER[row.rank] !== HEADLINE_TIER[best.rank])
        return HEADLINE_TIER[row.rank] > HEADLINE_TIER[best.rank] ? row : best;
      return row.standing.severity > best.standing.severity ? row : best;
    }, null);
  return ranked ?? rows.find((row) => row.value != null) ?? null;
}

/** "−98% vs median", or null when there is no honest comparison to make. */
function gapPhrase(row: CardRow): string | null {
  const { metric, value, standing } = row;
  const stats = standing.stats;
  if (
    value == null ||
    !standing.eligible ||
    stats == null ||
    Math.abs(standing.gapDelta) <= 1e-9
  )
    return null;
  const gap = formatGapMagnitude({
    value,
    median: stats.p50,
    gapPct: standing.gapPct,
    gapDelta: standing.gapDelta,
    format: metric.format,
    unit: metric.unit,
  });
  return gap == null ? null : `${gap} vs median`;
}
