/**
 * Metrics that count a superset of what another metric counts.
 *
 * Two metrics where one contains the other move together almost perfectly, so
 * a rule that picks out unusual values picks out both — and the reader meets
 * what looks like two findings ("Lines added 229" above "Code lines added 73")
 * and is left to work out that the second is part of the first.
 *
 * Keys are the BROADER metric; the values are what it already includes.
 * Relationships are taken from the catalog's own definitions, not inferred
 * from correlation — a pair that merely moves together is not a pair where one
 * contains the other.
 */
export const METRIC_CONTAINS: Readonly<Record<string, readonly string[]>> = {
  // "All lines added, by file category" ⊃ "lines added to code files".
  "git.lines_added": ["git.code_lines"],
  // "Files shared with any recipient" ⊃ inside / outside the organization.
  "collab.files_shared": [
    "collab.files_shared_internal",
    "collab.files_shared_external",
  ],
  // "Chat messages sent" ⊃ the ones posted to shared channels.
  "collab.messages_sent": ["collab.channel_posts"],
};

/**
 * Metrics that restate one another — the same fact written in different units.
 *
 * Not a containment: neither is part of the other, they are two ways of saying
 * one thing, and either alone says all of it. Reporting both puts two marks on
 * one fact, and a reader counts marks.
 *
 * Membership is arithmetic, taken from the catalog's definitions. A pair that
 * merely correlates does not belong here — correlation is a finding, identity
 * is a duplicate.
 */
export const METRIC_RESTATES: readonly (readonly string[])[] = [
  // "Share of the workday NOT spent in meetings" is one minus meeting hours
  // over the working day; meeting-free days counts the days where that share
  // is whole. Three readings of the same measurement.
  ["collab.focus_time_pct", "collab.meeting_hours", "collab.meeting_free_days"],
];

const RESTATEMENT_GROUP = new Map<string, number>(
  METRIC_RESTATES.flatMap((group, index) =>
    group.map((key) => [key, index] as const)
  )
);

/**
 * One fact, one row.
 *
 * Of a wider metric and a narrower one, the NARROWER survives: it is the more
 * specific claim — "lines added to code files" excludes documentation, tests
 * and configuration, so it says something the wider metric cannot. Dropping
 * the narrower instead would trade a precise finding for a vague one.
 *
 * `alreadyShown` is what a more prominent surface has already said — the
 * headline row above this block. A metric that restates one of those is not a
 * second finding, it is the same finding further down the page, and the two
 * are not even recognisable as the same measurement from their names.
 *
 * Order matters: of several restatements, the FIRST survives, so callers pass
 * items in the order the reader will meet them — most severe first, or in
 * candidate order for a row that is chosen rather than ranked.
 */
export function dropRedundantMetrics<T extends { key: string }>(
  items: readonly T[],
  alreadyShown: ReadonlySet<string> = new Set()
): T[] {
  const present = new Set(items.map((item) => item.key));
  const shownGroups = new Set(
    [...alreadyShown].flatMap((key) => {
      const group = RESTATEMENT_GROUP.get(key);
      return group == null ? [] : [group];
    })
  );
  const kept: T[] = [];
  for (const item of items) {
    const containsShown = [...(METRIC_CONTAINS[item.key] ?? [])].some(
      (narrower) => present.has(narrower) || alreadyShown.has(narrower)
    );
    if (containsShown) continue;
    const group = RESTATEMENT_GROUP.get(item.key);
    if (group != null) {
      if (shownGroups.has(group)) continue;
      shownGroups.add(group);
    }
    kept.push(item);
  }
  return kept;
}
