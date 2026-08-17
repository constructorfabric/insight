import { Link, useRouterState } from "@tanstack/react-router";
import { ChevronDown, ChevronRight, User, Users } from "lucide-react";
import { useMemo } from "react";

import { useViewer } from "@/auth";
import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import { usePortalNavActions } from "@/lib/portal/portal-nav";
import { personIdFromPath } from "@/lib/metrics/entity";
import {
  filterOrgTree,
  type OrgTreeFilter,
} from "@/lib/portal/org-tree-filter";
import { useIcPerson } from "@/queries/ic-dashboard";
import type { IdentityPerson } from "@/types/insight";

// Person ids, not emails: the identity cutover made the id the key the route
// segment, `?scope=` and the metric entity ids all carry.
function personIdEq(a: string, b: string): boolean {
  return a.toLowerCase() === b.toLowerCase();
}

function containsPerson(node: IdentityPerson, personId: string): boolean {
  if (personIdEq(node.person_id, personId)) return true;
  return node.subordinates.some((s) => containsPerson(s, personId));
}

function PersonNode({
  node,
  depth,
  activePersonId,
  leadsToTeam,
  filter,
}: {
  node: IdentityPerson;
  depth: number;
  activePersonId: string | null;
  /** Lead (has reports) links to their team roster instead of their own page. */
  leadsToTeam: boolean;
  filter: OrgTreeFilter | null;
}) {
  const { setScope } = usePortalNavActions();
  if (filter && !filter.visible.has(node.person_id)) return null;
  const hasReports = node.subordinates.length > 0;
  const isActive = activePersonId
    ? personIdEq(activePersonId, node.person_id)
    : false;
  const hasActiveDescendant =
    hasReports && activePersonId
      ? node.subordinates.some((s) => containsPerson(s, activePersonId))
      : false;
  // While filtering, the chain to a match is what the reader asked to see, so
  // it opens regardless of where they happen to be standing.
  const open = filter ? true : depth === 0 || isActive || hasActiveDescendant;
  // A lead's name lands on their team; an IC's on their own page. (The two
  // literal `to`s keep the typed router happy vs. a computed path.) Drilling
  // into a lead also *sets the org scope* (design §6) so the topbar badge and
  // every org zone follow the node you just clicked.
  const link =
    hasReports && leadsToTeam ? (
      <Link
        to="/ic/$person/team"
        params={{ person: node.person_id }}
        onClick={() => setScope({ root: node.person_id })}
      />
    ) : (
      <Link to="/ic/$person/personal" params={{ person: node.person_id }} />
    );
  return (
    <>
      <SidebarMenuItem>
        <SidebarMenuButton
          isActive={isActive}
          render={link}
          style={{ paddingLeft: `${0.5 + depth * 0.875}rem` }}
        >
          {hasReports ? (
            open ? (
              <ChevronDown />
            ) : (
              <ChevronRight />
            )
          ) : (
            <span className="w-4 shrink-0" />
          )}
          {hasReports ? <Users /> : <User />}
          <span className="truncate">{node.display_name || node.email}</span>
        </SidebarMenuButton>
      </SidebarMenuItem>
      {hasReports && open
        ? node.subordinates.map((sub) => (
            <PersonNode
              key={sub.person_id}
              node={sub}
              depth={depth + 1}
              activePersonId={activePersonId}
              leadsToTeam={leadsToTeam}
              filter={filter}
            />
          ))
        : null}
    </>
  );
}

/**
 * Recursive org-chart navigation, rooted at the viewer. Extracted from
 * AppSidebar so the portal shell's context pane can reuse the same tree
 * without duplicating the traversal / active-node logic.
 */
export function OrgTree({
  leadsToTeam = false,
  query = "",
}: { leadsToTeam?: boolean; query?: string } = {}) {
  const { personId: viewerPersonId } = useViewer();
  const viewerQ = useIcPerson(viewerPersonId ?? "");
  const viewer = viewerQ.data ?? null;
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const activePersonId = useMemo(() => {
    const fromPath = personIdFromPath(pathname);
    if (fromPath) return fromPath;
    if (pathname === "/" && viewerPersonId) return viewerPersonId;
    return null;
  }, [pathname, viewerPersonId]);
  const filter = useMemo(() => filterOrgTree(viewer, query), [viewer, query]);

  if (!viewer) return null;
  if (filter && filter.visible.size === 0) {
    return (
      <p className="px-4 py-2 text-sm text-muted-foreground">
        No one here matches “{query.trim()}”
      </p>
    );
  }

  return (
    <SidebarMenu>
      <PersonNode
        node={viewer}
        depth={0}
        activePersonId={activePersonId}
        leadsToTeam={leadsToTeam}
        filter={filter}
      />
    </SidebarMenu>
  );
}
