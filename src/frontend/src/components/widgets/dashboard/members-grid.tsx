import { useMemo, useState } from "react";
import { Link } from "@tanstack/react-router";
import { ArrowDown, ArrowUp } from "lucide-react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  PreviewCard,
  PreviewCardContent,
  PreviewCardTrigger,
} from "@/components/ui/preview-card";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useSettings } from "@/hooks/use-settings";
import { formatMetricValue } from "@/lib/format";
import {
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import {
  computeDelta,
  deltaStatus,
  formatTileDelta,
  type MetricDelta,
} from "@/lib/metrics/delta";
import { formatGapMagnitude } from "@/lib/metrics/gap";
import { derivePeerStanding } from "@/lib/metrics/peer-standing";
import {
  rankCounts,
  rankableCount,
  sectionStandingPhrase,
  type RankCounts,
} from "@/lib/scoring";
import type {
  MetricDirection,
  MetricFormat,
} from "@/api/metric-results-client";
import {
  applyFocus,
  PEER_CELL,
  PEER_FILL,
  PEER_LABEL,
  PEER_TEXT,
  type FocusMode,
  type PeerCohortLabel,
  type PeerStatusWithNeutral,
} from "@/lib/peers";
import { applyFocusStatus, STATUS_TEXT_CLASS } from "@/lib/status";
import { cn } from "@/lib/utils";
import { evidenceSelection } from "@/api/metric-drilldown-client";
import { useMetricEvidenceOptional } from "@/components/metric-evidence-context";

export interface MembersGridMember {
  /** Canonical person id — keys every metric lookup AND the IC link. */
  entityId: string;
  displayName: string;
}

export interface MembersGridProps {
  members: MembersGridMember[];
  /** Column metrics, in display order; keys absent from `byKey` are skipped. */
  metricKeys: readonly string[];
  /** Current-period value + peer standing per metric key. */
  byKey: Map<string, NormalizedMetricResult>;
  /** Previous-period value per metric key — drives the trend arrows. */
  previousByKey?: Map<string, NormalizedMetricResult>;
  /**
   * Optional triage facet: per-member standing counts across ALL groups.
   * When present each row carries a standing chip (behind / ahead / on par,
   * the section-card vocabulary) and the grid offers the "Most behind" sort
   * (the default).
   */
  countsByMember?: Map<string, RankCounts>;
  /** Optional per-member worst-standing metric label, shown under the chip. */
  worstByMember?: Map<string, string | null>;
  /**
   * Show the standing facet (chip + "worst:" line + Member-header sort).
   * Defaults to on when `countsByMember` is supplied. Set explicitly to
   * derive the facet from the grid's OWN cells (group-local) when no
   * cross-group counts are passed — e.g. the group drilldown.
   */
  showIssues?: boolean;
  /** Accessible summary of what the table shows. */
  caption: string;
  /** Names the peer cohort in the cell popovers ("department median"). */
  cohortLabel?: PeerCohortLabel;
  className?: string;
}

interface Column {
  /** Metric key — column identity + sort key. */
  key: string;
  /** Compact header text (server short label, falling back to the label). */
  heading: string;
  label: string;
  unit: string | null;
  format: MetricFormat;
  direction: MetricDirection;
  metric: NormalizedMetricResult;
}

function columnFor(metric: NormalizedMetricResult): Column {
  return {
    key: metric.metric_key,
    heading: metric.short_label ?? metric.label,
    label: metric.label,
    unit: metric.unit,
    format: metric.format,
    direction: metric.direction,
    metric,
  };
}

/**
 * Which end of the scale reads as "better", or `null` for a neutral metric
 * that carries no better/worse judgment. Never collapse `direction` to a
 * boolean: a neutral metric is a fact, not a score, and must not be tinted
 * or ranked as if higher were good.
 */
function betterWhenHigher(direction: MetricDirection): boolean | null {
  if (direction === "higher_is_better") return true;
  if (direction === "lower_is_better") return false;
  return null;
}

interface CellShape {
  col: Column;
  value: number | null;
  previous: number | null;
  /** Period-over-period move (computation-aware); null when unobservable. */
  delta: MetricDelta | null;
  status: PeerStatusWithNeutral;
  median: number | null;
  /** Divergence vs the cohort median, straight from the standing — feeds
   *  `formatGapMagnitude` in the popover. */
  gapPct: number | null;
  gapDelta: number;
  /** Outlier weight; picks the member's worst cell for the group-local facet. */
  severity: number;
  /** False when the source never observed this member — a zero-filled sum
   *  reads 0, but an unmeasured member has no value to show. */
  observed: boolean;
}

interface RowShape {
  member: MembersGridMember;
  cells: CellShape[];
  /** Cross-group standing counts when provided, else derived from the row's
   *  own cells. Drives the chip phrase and the "Most behind" sort. */
  counts: RankCounts;
  worstLabel: string | null;
}

type SortKey = "name" | "issues" | string;

interface SortState {
  key: SortKey;
  /** Flipped by clicking the active column a second time. */
  reversed: boolean;
}

/** Cell text: bare number — the unit lives in the header tooltip and the
 *  cell popover, so ten columns of "141 commits / 707 lines" don't shout. */
function cellDisplay(value: number | null, col: Column): string {
  if (value == null || !Number.isFinite(value)) return "—";
  return formatMetricValue(value, col.format, null);
}

function fullDisplay(value: number | null, col: Column): string {
  if (value == null || !Number.isFinite(value)) return "—";
  return formatMetricValue(value, col.format, col.unit);
}

/** True when the cell has nothing rankable — always sorts last, both ways. */
function cellMissing(cell: CellShape | undefined): boolean {
  return !cell?.observed || cell.value == null || !Number.isFinite(cell.value);
}

/**
 * Roster × metrics as a semantic table: one row per member (name links to
 * their IC view), one sortable column per metric. Every cell is the member's
 * own value, trend arrow vs the previous period, and colour from their
 * standing vs their OWN department cohort — nothing is pooled and members are
 * never scored against each other; sorting only orders the rows.
 */
export function MembersGrid({
  members,
  metricKeys,
  byKey,
  previousByKey,
  countsByMember,
  worstByMember,
  showIssues,
  caption,
  cohortLabel = "department",
  className,
}: MembersGridProps) {
  const { focusMode } = useSettings();
  const hasIssuesFacet = showIssues ?? countsByMember != null;
  const [sort, setSort] = useState<SortState>({
    key: "issues",
    reversed: false,
  });

  const toggleSort = (key: SortKey) => {
    setSort((current) =>
      current.key === key
        ? { key, reversed: !current.reversed }
        : { key, reversed: false }
    );
  };

  const columns = useMemo(
    () =>
      metricKeys.flatMap((key) => {
        const metric = byKey.get(key);
        return metric ? [columnFor(metric)] : [];
      }),
    [metricKeys, byKey]
  );

  const rows = useMemo<RowShape[]>(() => {
    return members.map((member) => {
      const cells = columns.map((col) => {
        const metric = byKey.get(col.key)!;
        const data = forEntity(metric, member.entityId);
        const standing = derivePeerStanding(metric.direction, {
          value: data.value,
          peer: data.peer,
        });
        const prevMetric = previousByKey?.get(col.key);
        const previous = prevMetric
          ? forEntity(prevMetric, member.entityId).value
          : null;
        // Period-over-period move via the shared, computation-aware delta
        // (percentage points for percent ratios, relative % otherwise) — the
        // same helper the KPI tiles use. Only meaningful for observed members.
        const delta = standing.observed
          ? computeDelta(
              data.value,
              previous,
              metric.computation,
              metric.format
            )
          : null;
        return {
          col,
          value: data.value,
          previous,
          delta,
          status: standing.rank,
          median: standing.stats?.p50 ?? null,
          gapPct: standing.gapPct,
          gapDelta: standing.gapDelta,
          severity: standing.severity,
          observed: standing.observed,
        };
      });
      // Group-local fallback for the standing facet: rank counts over THIS
      // grid's cells, and the worst trailing metric (highest severity). Used
      // when no cross-group counts are supplied.
      const below = cells.filter((cell) => cell.status === "bottom");
      const selfWorst =
        below.length > 0
          ? below.reduce((worst, cell) =>
              cell.severity > worst.severity ? cell : worst
            ).col.label
          : null;
      return {
        member,
        cells,
        counts:
          countsByMember?.get(member.entityId) ??
          rankCounts(cells.map((cell) => ({ row: cell, rank: cell.status }))),
        worstLabel: worstByMember?.get(member.entityId) ?? selfWorst,
      };
    });
  }, [members, columns, byKey, previousByKey, countsByMember, worstByMember]);

  const sortedRows = useMemo(() => {
    const copy = [...rows];
    const flip = sort.reversed ? -1 : 1;
    if (sort.key === "name") {
      copy.sort(
        (a, b) =>
          flip * a.member.displayName.localeCompare(b.member.displayName)
      );
    } else if (sort.key === "issues") {
      copy.sort(
        (a, b) =>
          flip * (b.counts.bottom - a.counts.bottom) ||
          a.member.displayName.localeCompare(b.member.displayName)
      );
    } else {
      const colIdx = columns.findIndex((c) => c.key === sort.key);
      const col = columns[colIdx];
      if (col) {
        copy.sort((a, b) => {
          const ac = a.cells[colIdx];
          const bc = b.cells[colIdx];
          // Members the source never measured sort last in either direction —
          // an unmeasured cell (or a zero-filled sum with no observation) is
          // not a best or worst score. Mirrors the "—" the cell renders.
          const aMissing = cellMissing(ac);
          const bMissing = cellMissing(bc);
          if (aMissing || bMissing) return Number(aMissing) - Number(bMissing);
          // First click ranks best-first: lower-is-better puts smallest first;
          // higher-is-better and neutral put largest first (neutral's order is
          // arbitrary but stable — it implies no "best"). A second click flips.
          const bestFirst =
            betterWhenHigher(col.direction) === false
              ? ac!.value! - bc!.value!
              : bc!.value! - ac!.value!;
          return flip * bestFirst;
        });
      }
    }
    return copy;
  }, [rows, sort, columns]);

  /**
   * The active-column sort order as a real value direction — drives both
   * `aria-sort` and the header arrow. A metric column's first click sorts
   * best-first, so the value direction depends on the metric's own
   * better/worse direction (lower-is-better ascends first); name/issues
   * ascend on the first click and descend on the flip.
   */
  const directionFor = (
    key: SortKey
  ): "ascending" | "descending" | undefined => {
    if (sort.key !== key) return undefined;
    // Issues sorts most-first, so its unflipped order is descending; name
    // sorts A→Z (ascending unflipped).
    if (key === "issues") return sort.reversed ? "ascending" : "descending";
    const col = columns.find((c) => c.key === key);
    if (!col) return sort.reversed ? "descending" : "ascending";
    const bestIsHigh = betterWhenHigher(col.direction) !== false;
    return bestIsHigh !== sort.reversed ? "descending" : "ascending";
  };

  // Member-level orderings (name / issues) live on the Member header; metric
  // orderings live on their own headers. This is the arrow the Member header
  // shows — whichever member order is active, if any.
  const memberDirection = directionFor("issues") ?? directionFor("name");
  const memberSortActive = sort.key === "issues" || sort.key === "name";

  return (
    <div className={cn("flex flex-col gap-3", className)}>
      <div className="overflow-x-auto">
        {/* Fixed layout so metric columns are uniform — auto layout sizes
            each column to its own content, making them ragged. The member
            column takes a fixed width; the metric columns split the rest
            equally, filling the container (w-full) up to the ultrawide cap.
            Fixed layout ignores cell min-widths, so the floor that keeps
            headers legible is an explicit table min-width — past it the
            wrapper scrolls instead of crushing columns. */}
        <table
          className="w-full max-w-[1600px] table-fixed border-separate border-spacing-1"
          style={{ minWidth: `${256 + columns.length * 108}px` }}
        >
          <caption className="sr-only">{caption}</caption>
          <thead>
            <tr>
              <th
                scope="col"
                aria-sort={memberDirection}
                className="sticky left-0 z-10 w-64 bg-card px-2 text-left"
              >
                {hasIssuesFacet ? (
                  // Two member orderings (issues / name) → a small header menu,
                  // so the roster-ordering control lives on the column it
                  // orders instead of a lone button.
                  <DropdownMenu>
                    <DropdownMenuTrigger
                      render={
                        <button
                          type="button"
                          className={cn(
                            "flex cursor-pointer items-center gap-1 text-xs font-medium tracking-wider text-muted-foreground uppercase transition-colors hover:text-foreground",
                            memberSortActive && "text-foreground"
                          )}
                        >
                          Member
                          <SortArrow direction={memberDirection} />
                        </button>
                      }
                    />
                    <DropdownMenuContent align="start">
                      <DropdownMenuItem onClick={() => toggleSort("issues")}>
                        Most behind
                      </DropdownMenuItem>
                      <DropdownMenuItem onClick={() => toggleSort("name")}>
                        Name
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                ) : (
                  // Only name to order by → a plain toggle like a metric header.
                  <button
                    type="button"
                    onClick={() => toggleSort("name")}
                    className={cn(
                      "flex cursor-pointer items-center gap-1 text-xs font-medium tracking-wider text-muted-foreground uppercase transition-colors hover:text-foreground",
                      sort.key === "name" && "text-foreground"
                    )}
                  >
                    Member
                    <SortArrow direction={directionFor("name")} />
                  </button>
                )}
              </th>
              {columns.map((col) => (
                <th
                  key={col.key}
                  scope="col"
                  aria-sort={directionFor(col.key)}
                  className="p-0"
                >
                  <ColumnHeader
                    col={col}
                    active={sort.key === col.key}
                    direction={directionFor(col.key)}
                    onClick={() => toggleSort(col.key)}
                  />
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {sortedRows.map((row) => (
              <MemberRow
                key={row.member.entityId}
                row={row}
                showIssues={hasIssuesFacet}
                cohortLabel={cohortLabel}
                focusMode={focusMode}
              />
            ))}
          </tbody>
        </table>
      </div>
      <Legend />
    </div>
  );
}

function MemberRow({
  row,
  showIssues,
  cohortLabel,
  focusMode,
}: {
  row: RowShape;
  /** True when the cross-group facet backs the chip; the fallback self-count
   *  still sorts but is already visible as the row's own cell colours. */
  showIssues: boolean;
  cohortLabel: PeerCohortLabel;
  focusMode: FocusMode;
}) {
  const { member, cells, counts, worstLabel } = row;
  // The section-card vocabulary: behind wins over ahead on mixed profiles,
  // "on par" only when nothing sticks out either way. The chip states the
  // count; the tint carries the judgment.
  const chipText = showIssues ? sectionStandingPhrase(counts) : null;
  const chipRank: PeerStatusWithNeutral =
    rankableCount(counts) === 0
      ? "neutral"
      : counts.bottom > 0
        ? "bottom"
        : counts.top > 0
          ? "top"
          : "in_pack";
  return (
    <tr>
      <th
        scope="row"
        className="sticky left-0 z-10 w-64 bg-card px-2 py-1 text-left font-normal"
      >
        <div className="flex min-h-12 flex-col justify-center gap-0.5">
          <div className="flex items-center gap-1.5">
            <Link
              to="/ic/$person/personal"
              params={{ person: member.entityId }}
              className="min-w-0 truncate text-sm font-medium leading-tight hover:underline"
            >
              {member.displayName}
            </Link>
            {chipText != null ? (
              <span
                className={cn(
                  "shrink-0 rounded-sm px-1.5 py-0.5 text-xs font-medium",
                  PEER_CELL[applyFocus(chipRank, focusMode)]
                )}
              >
                {chipText}
              </span>
            ) : null}
          </div>
          {showIssues && counts.bottom > 0 && worstLabel ? (
            <p className="truncate text-[11px] leading-tight text-muted-foreground">
              worst: {worstLabel}
            </p>
          ) : null}
        </div>
      </th>
      {cells.map((cell) => (
        <td key={cell.col.key} className="p-0 align-middle">
          <GridCell
            cell={cell}
            entityId={member.entityId}
            memberName={member.displayName}
            cohortLabel={cohortLabel}
            focusMode={focusMode}
          />
        </td>
      ))}
    </tr>
  );
}

function GridCell({
  cell,
  entityId,
  memberName,
  cohortLabel,
  focusMode,
}: {
  cell: CellShape;
  entityId: string;
  memberName: string;
  cohortLabel: PeerCohortLabel;
  focusMode: FocusMode;
}) {
  const focused = applyFocus(cell.status, focusMode);
  const evidenceContext = useMetricEvidenceOptional();
  const { col, value, previous, delta, median, observed } = cell;
  const evidence = col.metric.drilldown
    ? evidenceSelection(
        col.metric.selection,
        entityId,
        undefined,
        undefined,
        col.metric.breakdown?.dimensions
      )
    : null;
  // Show the trend arrow only when the delta rounds to a real change (the
  // KPI-tile suppression rule); direction from the sign, favorability from
  // the shared deltaStatus (neutral metric → muted, no good/bad).
  const deltaText = delta ? formatTileDelta(delta) : null;
  const showTrend = deltaText != null;
  const trendUp = delta != null && delta.value > 0;
  const TrendIcon = trendUp ? ArrowUp : ArrowDown;
  const trendTint = delta
    ? STATUS_TEXT_CLASS[
        applyFocusStatus(deltaStatus(delta, col.direction), focusMode)
      ]
    : "text-muted-foreground";
  const display = observed ? cellDisplay(value, col) : "—";
  const displayWithUnit = observed ? fullDisplay(value, col) : "—";
  // Divergence magnitude vs the median — same formatter as the KPI tiles
  // ("3.5×" / "−39%" / "−35 pp"). Driven purely by having a value, a median,
  // and a real gap: a disclosed median is a fact regardless of the metric's
  // direction, so this shows for neutral-direction metrics too (the quartile
  // verdict below is what direction gates, not the number).
  const gapText =
    value != null && median != null && Math.abs(cell.gapDelta) > 1e-9
      ? formatGapMagnitude({
          value,
          median,
          gapPct: cell.gapPct,
          gapDelta: cell.gapDelta,
          format: col.format,
          unit: col.unit,
        })
      : null;
  // Quartile verdict exists only for a real rank; "neutral" is the
  // no-comparison placeholder, already stated by the context line — never
  // render it as a second line.
  const showStanding = observed && cell.status !== "neutral";
  return (
    // PreviewCard is base-ui's hover-only surface — no click-to-open at all,
    // so a pressed cell can never pin its detail open.
    <PreviewCard>
      <PreviewCardTrigger
        delay={150}
        // Close immediately on mouse-out — the default 300ms lets the old
        // card linger while the next cell's opens, flashing two at once.
        closeDelay={0}
        render={
          <button
            type="button"
            onClick={() => {
              if (evidence) evidenceContext?.openEvidence(evidence, col.label);
            }}
            aria-label={
              observed
                ? `${memberName} — ${col.label}: ${displayWithUnit} — ${PEER_LABEL[focused]}`
                : `${memberName} — ${col.label}: not recorded`
            }
            className={cn(
              "flex h-12 w-full items-center justify-center rounded-sm px-3 text-sm font-medium tabular-nums transition hover:brightness-95",
              PEER_CELL[focused]
            )}
          >
            {/* Value stays centered in the cell; the trend arrow is anchored
                to the value's right edge (left-full) so it hugs the number
                instead of drifting to the cell border. */}
            <span className="relative inline-flex items-center">
              {display}
              {showTrend && value != null ? (
                <TrendIcon
                  className={cn("absolute left-full ml-0.5 size-3", trendTint)}
                  aria-hidden
                />
              ) : null}
            </span>
          </button>
        }
      />
      <PreviewCardContent className="w-64 p-3">
        <div className="flex flex-col gap-1">
          <p className="text-sm font-semibold">{col.label}</p>
          <p className="text-xs text-muted-foreground">{memberName}</p>
          <p
            className={cn(
              "mt-2 text-2xl font-semibold tabular-nums",
              PEER_TEXT[focused]
            )}
          >
            {displayWithUnit}
          </p>
          <p className="text-xs text-muted-foreground">
            {!observed ? (
              // No value for this member — can't be "at the median". Show the
              // cohort median as context only.
              median != null ? (
                `Not recorded · ${cohortLabel} median ${formatMetricValue(median, col.format, col.unit)}`
              ) : (
                "Not recorded"
              )
            ) : median == null ? (
              "No peer data"
            ) : gapText != null ? (
              <>
                <span className={cn("font-medium", PEER_TEXT[focused])}>
                  {gapText}
                </span>{" "}
                from {cohortLabel} median{" "}
                {formatMetricValue(median, col.format, col.unit)}
              </>
            ) : (
              `At the ${cohortLabel} median (${formatMetricValue(median, col.format, col.unit)})`
            )}
          </p>
          {showTrend && previous != null ? (
            <p className="mt-1 text-xs text-muted-foreground tabular-nums">
              vs previous period:{" "}
              <span className={cn("font-medium", trendTint)}>{deltaText}</span>{" "}
              (was {formatMetricValue(previous, col.format, col.unit)})
            </p>
          ) : null}
          {showStanding ? (
            <p className={cn("mt-1 text-xs font-medium", PEER_TEXT[focused])}>
              {PEER_LABEL[focused]}
            </p>
          ) : null}
        </div>
      </PreviewCardContent>
    </PreviewCard>
  );
}

/** Ascending → up arrow, descending → down arrow; nothing when inactive.
 *  Column headers pass absolute-positioning classes so the arrow sits beside
 *  the centered label without contributing width (no reflow as the active
 *  sort moves); inline callers (the sort button) pass nothing. */
function SortArrow({
  direction,
  className,
}: {
  direction: "ascending" | "descending" | undefined;
  className?: string;
}) {
  if (!direction) return null;
  const Icon = direction === "ascending" ? ArrowUp : ArrowDown;
  return <Icon className={cn("size-3 shrink-0", className)} aria-hidden />;
}

function ColumnHeader({
  col,
  active,
  direction,
  onClick,
}: {
  col: Column;
  active: boolean;
  direction: "ascending" | "descending" | undefined;
  onClick: () => void;
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type="button"
            onClick={onClick}
            className={cn(
              "flex h-9 w-full cursor-pointer items-center justify-center px-4 text-xs font-medium tracking-wider text-muted-foreground uppercase transition-colors hover:text-foreground",
              active && "text-foreground"
            )}
            aria-label={`${col.label} — sort by this column`}
          >
            {/* Label stays centered; the arrow hugs its right edge (left-full)
                like the value cell, rather than pinning to the column edge. */}
            <span className="relative inline-flex max-w-full items-center">
              <span className="truncate">{col.heading}</span>
              <SortArrow
                direction={direction}
                className="absolute left-full ml-0.5"
              />
            </span>
          </button>
        }
      />
      <TooltipContent side="top" className="max-w-56">
        <span className="flex flex-col gap-0.5 leading-snug">
          <span className="font-medium">{col.label}</span>
          <span className="text-background/70">
            {col.unit ? `${col.unit} · ` : ""}
            {betterWhenHigher(col.direction) === true
              ? "higher is better · "
              : betterWhenHigher(col.direction) === false
                ? "lower is better · "
                : ""}
            click to sort
          </span>
        </span>
      </TooltipContent>
    </Tooltip>
  );
}

function Legend() {
  return (
    <div className="flex flex-wrap items-center gap-4 text-xs text-muted-foreground">
      <LegendSwatch className={PEER_FILL.top}>Top 25%</LegendSwatch>
      <LegendSwatch className={PEER_FILL.in_pack}>On par</LegendSwatch>
      <LegendSwatch className={PEER_FILL.bottom}>Bottom 25%</LegendSwatch>
      <LegendSwatch className={PEER_FILL.neutral}>No peer data</LegendSwatch>
    </div>
  );
}

function LegendSwatch({
  className,
  children,
}: {
  className: string;
  children: React.ReactNode;
}) {
  return (
    <span className="inline-flex items-center gap-1.5">
      <span className={cn("inline-block size-3 rounded-sm", className)} />
      {children}
    </span>
  );
}
