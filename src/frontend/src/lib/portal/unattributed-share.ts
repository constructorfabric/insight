/**
 * Whether a lens reads the whole organisation, the only scope that can own
 * work no person reaches: an unassigned issue or an unbound account belongs to
 * no team, so a subtree has no share of it to show.
 */
export interface UnattributedScope {
  /** One person is read, not a roster. */
  personal: boolean;
  /** The lens is narrowed to one dimension value. */
  scoped: boolean;
  /** The backend serves the figure: a flat policy, or an admin viewer. */
  mayReadOrganisation: boolean;
  /** People the lens sums, the pivot included. */
  scopeMemberCount: number;
  /** Everyone the viewer may see. */
  visibleCount: number;
}

export function showsUnattributedShare(scope: UnattributedScope): boolean {
  if (scope.personal || scope.scoped || !scope.mayReadOrganisation) {
    return false;
  }

  return scope.visibleCount > 0 && scope.scopeMemberCount >= scope.visibleCount;
}
