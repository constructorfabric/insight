import { useMemo } from "react";

import { useViewer } from "@/auth";
import {
  findIdentityNode,
  flattenSubordinates,
  hasIndirectReports,
  scopeRosterToDirectReports,
  type RosterEntry,
} from "@/lib/insight/identity-tree";
import {
  type OrgScope,
} from "@/lib/portal/portal-store";
import {
  usePortalScope,
} from "@/lib/portal/portal-nav";
import type { PersonSummary } from "@/api/identity-client";
import { personDisplayName } from "@/lib/identities/person-display";
import { useIcPerson } from "@/queries/ic-dashboard";
import { useVisibilityPolicy } from "@/queries/identity-me";
import { useVisibleRoster } from "@/queries/visible-roster";
import type { IdentityPerson } from "@/types/insight";

/** One option in the scope picker: a manager, their depth, and their team size. */
export interface ManagerNode {
  /** Canonical person id — what links, `?scope=` and metric ids all carry. */
  person_id: string;
  name: string;
  depth: number;
  teamSize: number;
}

/** The scope resolved against the viewer's own subtree — the permission boundary. */
export interface ResolvedScope {
  /** The scope pivot (root manager node); null while the tree loads. */
  pivot: IdentityPerson | null;
  /** Everyone inside the scope (subtree or direct reports). */
  roster: RosterEntry[] | null;
  label: string;
  count: number;
  /** All manager nodes of the viewer's tree, for the ScopeSelect picker. */
  managerNodes: ManagerNode[];
  /** Whether directOnly can change anything at this pivot. */
  canDirectOnly: boolean;
}

/** What the scope is called when it is the organisation itself. */
export const WHOLE_ORG_LABEL = "Whole organisation";

/**
 * Scope resolution for an organisation with no reporting lines.
 *
 * There is one cohort — everyone the viewer may see — so there is no pivot to
 * pick and nothing for `directOnly` to narrow. The viewer stays in: this scope
 * counts the organisation, and an org-level head-count that changes with who
 * is looking disagrees with the roster listed right beside it (#2724).
 */
export function flatOrgScope(
  roster: readonly PersonSummary[] | null,
): ResolvedScope {
  if (!roster) {
    return {
      pivot: null,
      roster: null,
      label: WHOLE_ORG_LABEL,
      count: 0,
      managerNodes: [],
      canDirectOnly: false,
    };
  }

  const members: RosterEntry[] = roster.map((person) => ({
    person_id: person.person_id,
    email: person.email ?? "",
    display_name: person.display_name ?? "",
    username: person.username ?? "",
    // No reporting lines to name, and no depth to be at.
    supervisor_person_id: null,
    is_direct: false,
  }));

  return {
    pivot: null,
    roster: members,
    label: WHOLE_ORG_LABEL,
    count: members.length,
    managerNodes: [],
    canDirectOnly: false,
  };
}

/**
 * Pure scope resolution (design §6): pivot = scope.root within the viewer's
 * tree (permission boundary — identity only serves the viewer their subtree),
 * falling back to the viewer; roster = subtree, optionally direct-only.
 */
export function resolveScopeRoster(
  tree: IdentityPerson | null,
  viewerPersonId: string | null,
  scope: OrgScope,
): ResolvedScope {
  if (!tree) {
    return {
      pivot: null,
      roster: null,
      label: "",
      count: 0,
      managerNodes: [],
      canDirectOnly: false,
    };
  }
  // Person id, not email: since the identity cutover that is the only key the
  // tree, the routes and the metric entity ids agree on.
  const viewerNode =
    (viewerPersonId ? findIdentityNode(tree, viewerPersonId) : null) ?? tree;
  const pivot =
    (scope.root ? findIdentityNode(viewerNode, scope.root) : null) ?? viewerNode;
  const full = flattenSubordinates(pivot);
  const canDirectOnly = hasIndirectReports(full);
  const roster = scopeRosterToDirectReports(full, canDirectOnly && scope.directOnly);

  const managerNodes: ManagerNode[] = [];
  // One pass: each call returns its own subtree size, so a team size costs one
  // visit per node. Flattening per manager node re-walked that manager's whole
  // subtree — O(n · depth), which degrades on a deep reporting chain.
  //
  // The entry is pushed BEFORE recursing and its size filled in after, so the
  // picker keeps its depth-first outline order (a lead, then that lead's leads).
  const walk = (node: IdentityPerson, depth: number): number => {
    const entry =
      node.subordinates.length > 0
        ? {
            person_id: node.person_id,
            name: personDisplayName(node),
            depth,
            teamSize: 0,
          }
        : null;
    if (entry) managerNodes.push(entry);
    let size = 0;
    for (const sub of node.subordinates) size += 1 + walk(sub, depth + 1);
    if (entry) entry.teamSize = size;
    return size;
  };
  walk(viewerNode, 0);

  return {
    pivot,
    roster,
    label: personDisplayName(pivot),
    count: roster?.length ?? 0,
    managerNodes,
    canDirectOnly,
  };
}

/** The one hook every org zone uses to know WHO it is looking at. */
export function useOrgScope(): ResolvedScope & {
  isLoading: boolean;
  isError: boolean;
  refetch: () => void;
  /** The pivot's person id — the roster key every org query is built from. */
  pivotPersonId: string;
} {
  const { personId } = useViewer();
  const viewerQ = useIcPerson(personId ?? "");
  const scope = usePortalScope();
  const { isFlat } = useVisibilityPolicy();
  const flatRoster = useVisibleRoster(isFlat);

  const resolved = useMemo(
    () =>
      isFlat
        ? flatOrgScope(flatRoster.isPending ? null : flatRoster.roster)
        : resolveScopeRoster(viewerQ.data ?? null, personId, scope),
    [isFlat, flatRoster.isPending, flatRoster.roster, viewerQ.data, personId, scope],
  );

  // Under a flat policy the roster IS the org zones' subject, so its failure is
  // theirs; the identity tree still answers who the viewer is.
  return {
    ...resolved,
    isLoading: isFlat ? flatRoster.isPending : viewerQ.isLoading,
    isError: isFlat ? flatRoster.isError : viewerQ.isError,
    refetch: () => (isFlat ? flatRoster.retry() : void viewerQ.refetch()),
    pivotPersonId: resolved.pivot?.person_id ?? personId ?? "",
  };
}
