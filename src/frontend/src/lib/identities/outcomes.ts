/**
 * Folding several correction answers into one.
 *
 * The merge endpoint joins exactly two persons, so a case holding more than two
 * is decided by a short sequence of calls rather than one. The operator took ONE
 * decision, so they are owed one answer — and it has to keep the per-account
 * detail, because `refused` is the item-level state that says a decision did not
 * land.
 */
import type { CorrectionResponse } from "@/api/identity-client";

export function combineOutcomes(
  results: readonly CorrectionResponse[],
): CorrectionResponse {
  return {
    applied: results.reduce((sum, r) => sum + r.applied, 0),
    already_decided: results.reduce((sum, r) => sum + r.already_decided, 0),
    items: results.flatMap((r) => r.items),
    // Only a detach mints one, and a detach is never part of a sequence — but
    // carrying the last one that exists keeps the fold total rather than
    // silently dropping a field the caller may report.
    new_person_id: results.findLast((r) => r.new_person_id != null)
      ?.new_person_id,
  };
}

/** Accounts the server refused to move. */
export function refusedCount(result: CorrectionResponse): number {
  return result.items.filter((item) => item.outcome === "refused").length;
}

/**
 * Every account the call named ended up decided.
 *
 * Deliberately not "no refusals": the outcome field is an OPEN vocabulary — the
 * API documents further values it may grow — so a build that has never heard of
 * one must not read it as success and close over it. Counting the two that mean
 * "decided" against the items named is the form that stays true.
 */
export function fullyDecided(result: CorrectionResponse): boolean {
  return result.applied + result.already_decided === result.items.length;
}
