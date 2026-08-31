import { forEntity, type NormalizedMetricResult } from "@/lib/metrics/collection";

/** A bucket of the chart, opened up: its total and the people behind it. */
export interface BucketBreakdownRow {
  date: string;
  total: number;
  contributors: string[];
}

/**
 * The chart's own buckets, each with the people who contributed to it.
 *
 * Read from the per-entity series the line is already summed from, so it costs
 * no request: the reader who asks "who is this?" of a team total is asking
 * about data the page is holding.
 *
 * A measured zero is not a contribution — same rule the active-contributors
 * line draws.
 */
export function bucketBreakdown(
  key: string,
  byKey: Map<string, NormalizedMetricResult>,
  members: readonly { person_id: string; name: string }[],
): BucketBreakdownRow[] {
  const result = byKey.get(key);
  if (!result) return [];

  // Identity is the person id, never the display name: two colleagues called
  // the same thing are two contributors, and deduplicating on the name would
  // report one where the chart's line counts two.
  //
  // A person counted once per bucket however many readings they have in it:
  // one contributor with two positive points is one active contributor.
  const nameOf = new Map(members.map((m) => [m.person_id, m.name]));
  const buckets = new Map<string, { total: number; contributors: Set<string> }>();
  for (const member of members) {
    for (const s of forEntity(result, member.person_id).series) {
      for (const p of s.points) {
        const bucket = buckets.get(p.bucket_start) ?? {
          total: 0,
          contributors: new Set<string>(),
        };
        const value = p.value ?? 0;
        bucket.total += value;
        if (value > 0) bucket.contributors.add(member.person_id);
        buckets.set(p.bucket_start, bucket);
      }
    }
  }

  return [...buckets.entries()]
    .map(([date, bucket]) => ({
      date,
      total: bucket.total,
      contributors: [...bucket.contributors]
        .map((id) => nameOf.get(id) ?? id)
        .sort((a, b) => a.localeCompare(b)),
    }))
    .sort((a, b) => a.date.localeCompare(b.date));
}
