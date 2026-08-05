export type PeerStats = {
  p25: number
  p50: number
  p75: number
  min: number
  max: number
  n: number
}

export type PeerStatus = "top" | "in_pack" | "bottom"
export type PeerStatusWithNeutral = PeerStatus | "neutral"

export type PeerCohortLabel = "team" | "org" | "department"

export type FocusMode = "all" | "critical" | "rewards" | "neutral"

export function applyFocus(
  status: PeerStatusWithNeutral,
  mode: FocusMode,
): PeerStatusWithNeutral {
  if (mode === "all") return status
  if (mode === "critical") return status === "bottom" ? status : "neutral"
  if (mode === "rewards") return status === "top" ? status : "neutral"
  return "neutral"
}

// Heatmap cell fills. Soft tints of the semantic accents (not the solid
// button/alert fills) so a dense grid reads as a calm data view rather than a
// wall of alarms; the hue still signals top/bottom, the number stays crisp in
// the neutral foreground. Tune the /alpha to taste.
export const PEER_CELL: Record<PeerStatusWithNeutral, string> = {
  top: "bg-success/30 text-foreground",
  bottom: "bg-destructive/20 text-foreground",
  in_pack: "bg-secondary text-secondary-foreground",
  neutral: "bg-muted text-muted-foreground",
}

export const PEER_TEXT: Record<PeerStatusWithNeutral, string> = {
  top: "text-success",
  bottom: "text-destructive",
  in_pack: "text-foreground",
  neutral: "text-muted-foreground",
}

export const PEER_LABEL: Record<PeerStatusWithNeutral, string> = {
  top: "Top 25%",
  in_pack: "On par",
  bottom: "Bottom 25%",
  neutral: "No peer data",
}

export const PEER_FILL: Record<PeerStatusWithNeutral, string> = {
  top: "bg-success",
  bottom: "bg-destructive",
  in_pack: "bg-muted-foreground/40",
  neutral: "bg-muted-foreground/20",
}

export function peerStatusVsQuartiles(
  value: number,
  stats: Pick<PeerStats, "p25" | "p50" | "p75">,
  higherIsBetter: boolean
): PeerStatus {
  // An outlier must sit strictly on the outlier side of the MEDIAN as well
  // as beyond the quartile. Quartile boundaries alone overclaim on tie-heavy
  // pools: in an all-zero cohort everyone satisfies `value >= p75`, and in a
  // zero-inflated one (p25 == median == 0, a few sharers above) everyone at
  // zero satisfies `value <= p25` — branding half the pack "Top/Bottom 25%".
  // Requiring the median side also keeps quartile ranks consistent with the
  // median-based scorers (a "bottom" here is always below-median there).
  if (higherIsBetter) {
    if (value >= stats.p75 && value > stats.p50) return "top"
    if (value <= stats.p25 && value < stats.p50) return "bottom"
    return "in_pack"
  }
  if (value <= stats.p25 && value < stats.p50) return "top"
  if (value >= stats.p75 && value > stats.p50) return "bottom"
  return "in_pack"
}

