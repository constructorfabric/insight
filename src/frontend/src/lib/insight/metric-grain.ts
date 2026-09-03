import type { MetricEvidenceRow } from "@/api/metric-drilldown-client";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";

/**
 * How closely a metric can be looked at.
 *
 * Every metric declares this about itself in the results response, so a
 * section never has to be told per group what its detail looks like: it asks
 * each metric how closely it can be read and renders that. A metric that
 * gains detail later gains it on screen without a layout change, and one that
 * has none says so instead of being drawn as though it did.
 */
export type MetricGrain = "event" | "derived_population" | "source_summary";

/** Closest first — what a metric can show at best. */
const GRAIN_ORDER: MetricGrain[] = [
  "event",
  "derived_population",
  "source_summary",
];

/**
 * The closest look a metric offers, or null when it offers none.
 *
 * `event` names the things themselves. `derived_population` names the daily
 * readings a ratio was built from, and carries both sides of it — for a share
 * of the day that is the most explanatory of the three, because the argument
 * a reader has with a percentage is almost always about its denominator.
 * `source_summary` is a daily counter and can say when, not what.
 */
export function finestGrain(
  metric: NormalizedMetricResult | undefined
): MetricGrain | null {
  const declared = metric?.drilldown?.granularity ?? [];
  return GRAIN_ORDER.find((grain) => declared.includes(grain)) ?? null;
}

export interface DayReading {
  date: string;
  value: number;
  /** Both sides of a ratio, when the metric reports them. */
  numerator: number | null;
  denominator: number | null;
}

function num(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export interface ActivityEvent {
  /** Stable identity of the thing — a commit hash, a page id. */
  ref: string | null;
  /** What it is called. Absent for sources that report no title. */
  title: string | null;
  /** Where it happened — a repository, a space. */
  context: string | null;
  date: string;
  /** The metric's own reading for this event, when it has one. */
  value: number | null;
  links: Readonly<Record<string, string>>;
  values: Readonly<Record<string, unknown>>;
}

/** Everything after the first line — a commit body, not its subject. */
function firstLine(title: string): string {
  return title.split("\n", 1)[0]?.trim() ?? "";
}

/**
 * The things themselves, newest first.
 *
 * Only the first line of a title survives. A commit message carries its
 * reasoning in the body, and that belongs in the export or the dialog, not in
 * a list a reader is scanning.
 */
export function activityEvents(rows: MetricEvidenceRow[]): ActivityEvent[] {
  return rows
    .flatMap((row) => {
      const date = row.values.date;
      if (typeof date !== "string") return [];
      const title = row.values.title;
      const context = row.values.repository ?? row.values.space;
      const ref = row.values.ref;
      return [
        {
          ref: typeof ref === "string" ? ref : null,
          title: typeof title === "string" ? firstLine(title) || null : null,
          context: typeof context === "string" ? context : null,
          date,
          value: num(row.values.value),
          links: row.links ?? {},
          values: row.values,
        },
      ];
    })
    .sort((a, b) => b.date.localeCompare(a.date));
}
