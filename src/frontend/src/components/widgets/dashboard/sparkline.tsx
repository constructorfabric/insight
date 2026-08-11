import { cn } from "@/lib/utils";

export interface SparklineProps {
  /** Readings oldest first; null is a gap, never a zero. */
  points: (number | null)[];
  className?: string;
}

const W = 72;
const H = 16;

/**
 * Where a number has been, drawn beside where it is now.
 *
 * Scaled to its own range rather than to zero or to any shared axis: the tile
 * already carries the magnitude in figures, so the only thing left for the
 * line to say is which way and how steadily. A missing reading breaks the line
 * instead of dropping it to the floor — a month with no data must not be
 * readable as a month with none of the thing.
 */
export function Sparkline({ points, className }: SparklineProps) {
  // What is worth drawing is the caller's call (`personTrendPoints`); this
  // guard only keeps the maths safe for a series that cannot be scaled.
  const values = points.filter((v) => v != null);
  if (values.length < 2) return null;

  const min = Math.min(...values);
  const max = Math.max(...values);
  const span = max - min || 1;
  const x = (i: number) => (i / (points.length - 1)) * W;
  const y = (v: number) => H - ((v - min) / span) * H;

  const segments: string[] = [];
  let run: string[] = [];
  points.forEach((v, i) => {
    if (v == null) {
      if (run.length > 1) segments.push(run.join(" "));
      run = [];
      return;
    }
    run.push(`${run.length ? "L" : "M"}${x(i).toFixed(1)},${y(v).toFixed(1)}`);
  });
  if (run.length > 1) segments.push(run.join(" "));

  const lastIndex = points.findLastIndex((v) => v != null);
  const lastValue = points[lastIndex];

  return (
    <svg
      data-slot="sparkline"
      viewBox={`0 0 ${W} ${H + 3}`}
      width={W}
      height={H + 3}
      className={cn("overflow-visible text-muted-foreground", className)}
      aria-hidden
    >
      {/* A rule under the line, spanning the whole window.
          Without it the line floats: nothing says where the span starts or
          ends, so a reader cannot tell a line covering the window from one
          that happens to sit at that angle. It claims no VALUE — the line is
          scaled to its own range, and a baseline at zero would be a lie for a
          percentage — only the extent of time. */}
      <line
        x1={0}
        y1={H + 2}
        x2={W}
        y2={H + 2}
        stroke="currentColor"
        strokeWidth={0.75}
        className="text-muted-foreground"
      />
      {/* Ticks at the ends, so the span reads as a span. */}
      <line
        x1={0}
        y1={H}
        x2={0}
        y2={H + 3}
        stroke="currentColor"
        strokeWidth={0.75}
        className="text-muted-foreground"
      />
      <line
        x1={W}
        y1={H}
        x2={W}
        y2={H + 3}
        stroke="currentColor"
        strokeWidth={0.75}
        className="text-muted-foreground"
      />
      {segments.map((d) => (
        <path
          key={d}
          d={d}
          fill="none"
          stroke="currentColor"
          strokeWidth={1.25}
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      ))}
      {/* The end of the line marked, so the eye knows which end is now. */}
      {lastValue != null ? (
        <circle
          cx={x(lastIndex)}
          cy={y(lastValue)}
          r={1.5}
          fill="currentColor"
        />
      ) : null}
    </svg>
  );
}
