import type { PeerStatusWithNeutral } from "./peers"
import type { Status } from "./status"

export type RankedMetric<T> = {
  row: T
  rank: PeerStatusWithNeutral
}

export type RankCounts = {
  top: number
  inPack: number
  bottom: number
  unranked: number
}

export function rankCounts<T>(metrics: RankedMetric<T>[]): RankCounts {
  const c: RankCounts = { top: 0, inPack: 0, bottom: 0, unranked: 0 }
  for (const m of metrics) {
    if (m.rank === "top") c.top++
    else if (m.rank === "in_pack") c.inPack++
    else if (m.rank === "bottom") c.bottom++
    else c.unranked++
  }
  return c
}

export function rankableCount(counts: RankCounts): number {
  return counts.top + counts.inPack + counts.bottom
}

/**
 * Whether a rank carries a comparison at all — the same set `rankCounts`
 * tallies as ranked. A metric present in the response but unmeasured for this
 * person, or measured against a pool too thin to disclose, ranks `neutral`: it
 * is not a weak reading, it is no reading.
 */
export function isRankable(rank: PeerStatusWithNeutral | null): boolean {
  return rank === "top" || rank === "in_pack" || rank === "bottom"
}

/**
 * Fraction of the rankable set a rank must clear to count as a section-wide
 * pattern rather than quartile noise. A quartile hands ~25% of any healthy
 * cohort to the bottom by construction, so a lone weak metric is expected, not
 * a signal.
 */
const SECTION_PATTERN_BAR = 0.25

/**
 * Section stripe/dot status from its rankable metrics — symmetric and
 * base-rate aware. Red demands a pattern of weakness (≥2 bottoms AND more than
 * a quarter of the rankable set); green mirrors it for strength and yields to
 * any red. A single bottom below the red bar is amber; an all-in-pack or
 * unrankable section stays calm (neutral). `in_pack` never pushes toward amber
 * on its own — being with the pack is the normal state, not a warning.
 */
export function gradeSectionStanding(counts: RankCounts): Status {
  const rankable = rankableCount(counts)
  if (rankable === 0) return "neutral"
  if (counts.bottom >= 2 && counts.bottom / rankable > SECTION_PATTERN_BAR)
    return "bad"
  if (counts.bottom >= 1) return "warn"
  if (counts.top >= 2 && counts.top / rankable > SECTION_PATTERN_BAR)
    return "good"
  return "neutral"
}

/**
 * Factual one-liner for the section badge — the text states a count, the color
 * (from `gradeSectionStanding`) carries the judgment. "Behind" wins over
 * "ahead" on mixed profiles: the badge is a triage signal, and a strength
 * never prompts opening a card.
 *
 * The count says "N of M" because it covers the whole SECTION while the card
 * below it shows a few rows. Left as a bare "5 behind peers" it read as a
 * claim about those rows and contradicted them: a card could say "4 ahead of
 * peers" over four rows with no green mark on any of them, and there was no
 * way for the reader to tell the badge was counting further than they could
 * see.
 */
export function sectionStandingPhrase(counts: RankCounts): string {
  const rankable = rankableCount(counts)
  if (rankable === 0) return "no comparison"
  if (counts.bottom > 0) return `${counts.bottom} of ${rankable} behind peers`
  if (counts.top > 0) return `${counts.top} of ${rankable} ahead of peers`
  return "near the median"
}

