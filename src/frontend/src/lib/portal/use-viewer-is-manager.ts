import { useMemo } from "react";

import { useViewer } from "@/auth";
import { findIdentityNode } from "@/lib/insight/identity-tree";
import { useIcPerson } from "@/queries/ic-dashboard";

/**
 * Whether the current viewer manages anyone. The portal's org zones (Overview,
 * People, Directions, AI & Cost, …) all roll up the viewer's subtree, so they
 * only mean something for a manager. An individual contributor has no subtree,
 * so those zones would be empty — the shell collapses to their Person page
 * instead (see LensRail / PortalLayout).
 *
 * `isPending` is true only until the viewer's own identity resolves; callers
 * should treat pending as "assume manager" to avoid hiding zones on a flash.
 */
export function useViewerIsManager(): { isManager: boolean; isPending: boolean } {
  const { personId } = useViewer();
  const q = useIcPerson(personId ?? "");

  const isManager = useMemo(() => {
    const tree = q.data ?? null;
    if (!tree || !personId) return false;
    const node = findIdentityNode(tree, personId) ?? tree;
    return (node.subordinates?.length ?? 0) > 0;
  }, [q.data, personId]);

  // `q.data == null` covers the error case too: with no tree we do not know,
  // and callers treat unresolved as "assume manager" so the org zone can show
  // the identity failure instead of the shell quietly demoting the viewer.
  return { isManager, isPending: q.isPending || q.data == null };
}
