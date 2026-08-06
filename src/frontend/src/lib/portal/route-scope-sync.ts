/**
 * The one-shot guard behind the route → org-scope sync.
 *
 * Module-scoped, deliberately NOT a ref: the guard has to outlive the People
 * view. A per-mount ref would re-fire the sync every time the reader leaves the
 * zone and comes back, silently reverting a scope they had picked in the topbar.
 * Keyed by the LATEST person, not by a set of every person ever seen: arriving
 * at a person's team is a navigation, and the reader who clicks "A's team"
 * after visiting B has to get A's roster. Remembering every person would leave
 * the scope on B while the route said A — a disagreement between the address
 * and the screen, which is what this whole migration was about avoiding. The
 * guard's job is narrower: absorb re-renders and remounts of the SAME person.
 *
 * It lives in its own module for two reasons: a component file cannot export
 * helpers without breaking fast refresh, and tests need the reset — vitest gives
 * one module registry per file, so without it the first case that mounts a given
 * person consumes the guard and every later case sees no sync at all.
 */
let lastSynced: string | null = null;

/**
 * True at most once per person: the caller may sync the scope, and the guard
 * records that it did.
 */
export function claimScopeSync(personId: string): boolean {
  if (!personId || lastSynced === personId) return false;
  lastSynced = personId;
  return true;
}

/** Tests only — see the note above. */
export function resetRouteScopeSync(): void {
  lastSynced = null;
}
