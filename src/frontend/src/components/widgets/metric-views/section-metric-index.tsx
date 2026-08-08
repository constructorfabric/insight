import { MetricName } from "@/components/widgets/metric-help-tooltip";
import {
  PEER_MARK_CENTRE,
  PeerMark,
} from "@/components/widgets/metric-views/peer-mark";
import { formatMetricValue } from "@/lib/format";
import { metricComparisons } from "@/lib/insight/metric-comparison";
import { derivePeerStanding } from "@/lib/metrics/peer-standing";
import {
  forEntity,
  type MetricCollectionConfig,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";

/**
 * Everything else the section measures, named and valued.
 *
 * A section shows a few metrics closely, and the rest of its collection would
 * otherwise be invisible — a person could not find out that the thing was
 * measured at all, let alone what it read. The list that used to carry them
 * was folded behind "supporting and on-par metrics", which sorted them by
 * their standing against a cohort: a reader looking for emails sent had to
 * know it was unremarkable this period in order to guess where it went.
 *
 * So: no ranking, no fold, alphabetical order, the value, the pool's middle,
 * and the catalog's own words on hover.
 *
 * Only rows that read. A metric holding nothing for this person in this
 * period gives the reader nothing to act on and costs them a line to discover
 * that; a column of dashes is the fastest way to teach someone to stop
 * reading a list. Why a metric is silent — a source nobody wired, or a period
 * this person did none of it — is a question about the section, and belongs
 * once at the top rather than nineteen times down its body.
 */
export function SectionMetricIndex({
  collection,
  byKey,
  entityId,
  shown,
}: {
  collection: MetricCollectionConfig;
  byKey: Map<string, NormalizedMetricResult>;
  entityId: string;
  /** Keys already given their own block above. */
  shown: ReadonlySet<string>;
}) {
  // Alphabetical, because the only thing a reader does with this list is look
  // something up. Collection order is the order the metrics were declared in
  // and means nothing on screen; ordering by value or by standing would rank
  // them, which is the judgment this section deliberately withholds — and
  // would also make a named metric impossible to find without reading all of
  // them.
  const rest = collection.metrics
    .flatMap((m) => {
      if (shown.has(m.key)) return [];
      const metric = byKey.get(m.key);
      if (!metric) return [];
      // A row that reads "—" against "median —" tells the reader nothing they
      // can use and costs them a line to find that out. Whether the silence
      // is a missing connector or a period this person did none of it is a
      // question about the section, answered once at the top, not nineteen
      // times down a list.
      return forEntity(metric, entityId).value == null ? [] : [metric];
    })
    .sort((a, b) => a.label.localeCompare(b.label));
  if (rest.length === 0) return null;

  return (
    <section className="rounded-xl border p-4 sm:p-5">
      <h2 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
        Also measured here
      </h2>
      {/* One column, whatever the width. Side-by-side columns of aligned
          names and numbers read as a table — the eye takes the row first — so
          an alphabet running down each column is invisible, and the list
          looks like a heap however carefully it was sorted. Vertical space at
          the very bottom of a page is the cheapest thing here. */}
      {/* The axis middle, drawn once behind every mark. Per-row ticks would
          break at each row's padding and read as a dotted column; the whole
          point is an unbroken line for the ordinary readings to disappear
          into. */}
      <dl className="relative pt-2">
        <span
          aria-hidden
          className="pointer-events-none absolute inset-y-2 w-px bg-foreground/20"
          style={{ right: PEER_MARK_CENTRE }}
        />
        {rest.map((metric) => (
          <div
            key={metric.metric_key}
            className="flex items-center justify-between gap-4 border-b border-dashed py-1 last:border-b-0"
          >
            <dt className="min-w-0 flex-1 truncate text-xs">
              <MetricName metric={metric} />
            </dt>
            <dd className="flex shrink-0 items-baseline gap-2 text-xs tabular-nums">
              <span className="w-32 text-right">
                {formatMetricValue(
                  forEntity(metric, entityId).value,
                  metric.format,
                  metric.unit
                )}
              </span>
              {/* The pool's middle, so a row is readable without opening
                  anything. Uncoloured: a list of thirty numbers lit up by
                  quartile is a scoreboard, and the reader did not ask to be
                  scored on every one of them. */}
              <span className="w-28 text-right text-muted-foreground">
                {metricComparisons(metric, null, entityId).median ?? ""}
              </span>
              {/* And how far off that middle, on the axis every row shares.
                  The number answers "what do the others have"; the mark
                  answers "how unlike them am I", which is the question a
                  reader cannot do in their head nineteen times. */}
              <PeerMark
                standing={derivePeerStanding(
                  metric.direction,
                  forEntity(metric, entityId)
                )}
                metricLabel={metric.label}
                format={metric.format}
                unit={metric.unit}
              />
            </dd>
          </div>
        ))}
      </dl>
    </section>
  );
}
