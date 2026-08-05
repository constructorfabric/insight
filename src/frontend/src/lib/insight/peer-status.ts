/**
 * Peer-status helpers (Refs #80).
 *
 * Render rule: a rank of top/bottom quartile colors; being with the pack is
 * the normal state and renders calm (neutral), never amber.
 */

import type { PeerStatusWithNeutral } from "@/lib/peers";
import type { Status } from "@/lib/status";

/**
 * THE rank → display-status mapping, shared by every surface that paints a
 * peer standing. Red means bottom quartile, green means top quartile, and
 * being with the pack is the normal state — it renders calm (neutral), never
 * amber. Amber is reserved for aggregate judgments that earn it (section
 * grading).
 */
export function peerStatusToStatus(p: PeerStatusWithNeutral): Status {
  if (p === "top") return "good";
  if (p === "bottom") return "bad";
  return "neutral";
}
