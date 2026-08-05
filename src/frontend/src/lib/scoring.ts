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
 */
export function sectionStandingPhrase(counts: RankCounts): string {
  if (rankableCount(counts) === 0) return "no peer data"
  if (counts.bottom > 0) return `${counts.bottom} behind peers`
  if (counts.top > 0) return `${counts.top} ahead of peers`
  return "on par with peers"
}

