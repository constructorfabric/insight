import type { MetricFormat } from "@/api/metric-results-client";
import { formatMetricValue } from "@/lib/format";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { peerSpread, type PeerStanding } from "@/lib/metrics/peer-standing";
import { cn } from "@/lib/utils";

/** Matches the metric-name tooltips beside it, so a sweep stays quiet. */
const HOVER_DELAY_MS = 400;

/** Half the axis, in cohort spreads. Beyond this a mark pins to the edge. */
const MAX_SPREADS = 2.5;

const W = 112;
const H = 20;
const PAD = 5;

/**
 * Distance from a row's right edge to the axis middle, for the list that
 * draws the shared centre line. Ties the rule to this component's geometry so
 * the two cannot drift apart.
 */
export const PEER_MARK_CENTRE = W / 2;

/** Where a deviation of `spreads` sits on the axis. */
function x(spreads: number): number {
  const clamped = Math.max(-MAX_SPREADS, Math.min(MAX_SPREADS, spreads));
  return W / 2 + (clamped / MAX_SPREADS) * (W / 2 - PAD);
}

/**
 * How far from the middle of the pool one reading sits, on an axis every
 * metric shares.
 *
 * A list of numbers each followed by its median asks the reader to do the
 * subtraction nineteen times, and a per-metric bar asks them to learn a new
 * scale on every row. Measuring the distance in cohort spreads instead —
 * `(value − median) / IQR` — puts hours, messages and percentages on one axis,
 * because "eight tenths of a spread below the middle" means the same thing
 * whatever is being counted.
 *
 * The point of the shared axis is the vertical line it makes — drawn once by
 * the list behind every mark (`PEER_MARK_CENTRE`), not per row, because a line
 * broken by row padding is just a column of dashes and the effect dies.
 * Everything ordinary settles onto that line. The blending is deliberate: "I
 * am like the others here" is the answer that should cost no attention, and
 * what the eye is left with is the few marks standing off it.
 *
 * Lower on the left, higher on the right — never flipped by direction.
 * Orienting the axis so that "left" means "worse" would smuggle a verdict into
 * the geometry, and for a metric with no better direction there is no worse to
 * point at.
 *
 * Colour is the one place a verdict is allowed, and only at its narrowest.
 * Standing out is not the same as being wrong: eleven spreads above the middle
 * is remarkable on any metric and a problem on almost none. So a mark is grey
 * unless the shared standing calls this reading bottom-of-pack on a metric
 * that HAS a wrong direction — the same call the overview makes, in a dot
 * instead of a red card. Two coloured marks in a list of nineteen is a
 * finding; nineteen coloured marks is a scoreboard, and the reader did not ask
 * to be scored on every row.
 */
export function PeerMark({
  standing,
  metricLabel,
  format,
  unit,
  className,
}: {
  standing: PeerStanding;
  metricLabel: string;
  /** So the tooltip can state the pool in the units the reader is reading. */
  format: MetricFormat;
  unit?: string | null;
  className?: string;
}) {
  const show = (value: number) => formatMetricValue(value, format, unit);
  // No mark where there is nothing to compare against: too few peers, a pool
  // with no spread, or a reading the peer view calls unmeasured. An empty
  // track still draws, so the line stays continuous and the row keeps its
  // height — an absent dot reads as "not comparable", not as "at the middle".
  const spreads =
    standing.eligible && standing.stats
      ? standing.gapDelta / peerSpread(standing.stats)
      : null;
  const pinned = spreads != null && Math.abs(spreads) > MAX_SPREADS;
  // The overview's own verdict, reused rather than re-derived: a section and
  // the page that sent the reader to it must not disagree about what counts.
  const adverse = standing.rank === "bottom";
  const markClass = adverse ? "text-destructive" : "text-muted-foreground";

  const svg = (
    <svg
      data-slot="peer-mark"
      viewBox={`0 0 ${W} ${H}`}
      width={W}
      height={H}
      className={cn("shrink-0 overflow-visible", className)}
      role="img"
      aria-label={describe(spreads, metricLabel)}
    >
      {/* One spread out, either way — a scale for "far", marked at the foot so
          it never competes with the reading itself. */}
      {[-1, 1].map((side) => (
        <line
          key={side}
          x1={x(side)}
          y1={H - 3}
          x2={x(side)}
          y2={H}
          stroke="currentColor"
          strokeWidth={1}
          className="text-muted-foreground"
        />
      ))}
      {spreads == null ? null : pinned ? (
        // Pinned rather than plotted: one metric sitting twelve spreads out
        // would otherwise squash the axis for the other eighteen. The shape
        // changes so the edge is not read as a measurement.
        <path
          d={
            spreads > 0
              ? `M${W - PAD - 4},${H / 2 - 3.5} L${W - PAD + 1},${H / 2} L${W - PAD - 4},${H / 2 + 3.5} Z`
              : `M${PAD + 4},${H / 2 - 3.5} L${PAD - 1},${H / 2} L${PAD + 4},${H / 2 + 3.5} Z`
          }
          fill="currentColor"
          className={markClass}
        />
      ) : (
        <circle
          cx={x(spreads)}
          cy={H / 2}
          r={3}
          fill="currentColor"
          className={markClass}
        />
      )}
    </svg>
  );

  // A mark says "far from the middle" and nothing about how far in the units
  // the reader thinks in. The tooltip closes that: the two readings and how
  // many people the middle was taken over, since a middle of four is a
  // different claim from a middle of forty.
  return (
    <TooltipProvider delay={HOVER_DELAY_MS}>
      <Tooltip>
        <TooltipTrigger render={<span className="inline-flex">{svg}</span>} />
        <TooltipContent
          side="top"
          className="flex-col items-start gap-1 text-left leading-relaxed"
        >
          <p>{describe(spreads, metricLabel)}</p>
          {standing.stats ? (
            <p className="text-background/70">
              {standing.stats.n} compared · middle {show(standing.stats.p50)} ·
              middle half {show(standing.stats.p25)}–{show(standing.stats.p75)}
            </p>
          ) : null}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

/** What the mark says, for anyone not reading it with their eyes. */
function describe(spreads: number | null, metricLabel: string): string {
  if (spreads == null) return `${metricLabel}: nothing comparable in the group`;
  const size = Math.abs(spreads).toFixed(1);
  const side = spreads > 0 ? "above" : "below";
  if (Math.abs(spreads) < 0.05) return `${metricLabel}: at the group's middle`;
  return `${metricLabel}: ${size} spreads ${side} the group's middle`;
}
