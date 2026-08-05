import type { FocusMode } from "./peers"

export type Status = "good" | "warn" | "bad" | "neutral"

export function applyFocusStatus(status: Status, mode: FocusMode): Status {
  if (mode === "all") return status
  if (mode === "critical") return status === "bad" ? status : "neutral"
  if (mode === "rewards") return status === "good" ? status : "neutral"
  return "neutral"
}

export const STATUS_BG_CLASS: Record<Status, string> = {
  good: "bg-success",
  warn: "bg-warning",
  bad: "bg-destructive",
  neutral: "bg-muted-foreground/40",
}

export const STATUS_TEXT_CLASS: Record<Status, string> = {
  good: "text-success",
  warn: "text-warning",
  bad: "text-destructive",
  neutral: "text-muted-foreground",
}

export const STATUS_SURFACE_CLASS: Record<Status, string> = {
  good: "bg-success/15 text-success",
  warn: "bg-warning/15 text-warning",
  bad: "bg-destructive/15 text-destructive",
  neutral: "bg-muted text-muted-foreground",
}

/**
 * The status stripe along a card edge — the ONE stripe token every card
 * uses, so stripe width never drifts between surfaces. An inset shadow, not
 * a border: it follows the card's corner radius and cannot stack against
 * the Card ring into a thicker edge. The edge is the per-surface choice
 * (top for the peer-story hero, left for everything else); the width is
 * not. Neutral carries no stripe — a stripe that says nothing is chrome.
 * Full literal class strings so Tailwind's scanner sees them.
 */
export const STATUS_STRIPE_LEFT: Record<Status, string | undefined> = {
  good: "shadow-[inset_3px_0_0_0_var(--success)]",
  warn: "shadow-[inset_3px_0_0_0_var(--warning)]",
  bad: "shadow-[inset_3px_0_0_0_var(--destructive)]",
  neutral: undefined,
}

export const STATUS_STRIPE_TOP: Record<Status, string | undefined> = {
  good: "shadow-[inset_0_3px_0_0_var(--success)]",
  warn: "shadow-[inset_0_3px_0_0_var(--warning)]",
  bad: "shadow-[inset_0_3px_0_0_var(--destructive)]",
  neutral: undefined,
}

/**
 * Value-form status colors for contexts where a Tailwind class can't be used —
 * e.g. SVG `fill` on recharts cells. Mirrors the class maps above so the two
 * stay in lockstep.
 */
export const STATUS_COLOR_VAR: Record<Status, string> = {
  good: "var(--success)",
  warn: "var(--warning)",
  bad: "var(--destructive)",
  neutral: "var(--muted-foreground)",
}

/** Score a value against a reference median, given metric direction. */
export function statusVsMedian(
  value: number,
  median: number,
  higherIsBetter: boolean
): Status {
  // A median of 0 is a legitimate comparison point (e.g. a low-activity
  // cohort); "no comparison" is expressed as a null median upstream, not 0.
  if (!Number.isFinite(value) || !Number.isFinite(median)) {
    return "neutral"
  }
  return (higherIsBetter ? value >= median : value <= median) ? "good" : "bad"
}

