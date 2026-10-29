import { Link, useRouterState } from "@tanstack/react-router";
import { ChevronDown, ChevronRight, User, Users } from "lucide-react";
import { useMemo, useState } from "react";

import type { PeopleListItem } from "@/api/identity-client";
import { useViewer } from "@/auth";
import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import { personName } from "@/lib/identities/person-display";
import { usePortalNavActions } from "@/lib/portal/portal-nav";
import { personIdFromPath } from "@/lib/metrics/entity";
import {
  filterOrgTree,
  type OrgTreeFilter,
} from "@/lib/portal/org-tree-filter";
import { rosterTree } from "@/lib/insight/identity-tree";
import { useVisibilityPolicy } from "@/queries/identity-me";
import { useVisibleRoster } from "@/queries/visible-roster";
import type { IdentityPerson } from "@/types/insight";

const UNNAMED_PERSON = "Unnamed person";

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
  expansionOverrides,
  onExpandedChange,
}: {
  node: IdentityPerson;
  depth: number;
  activePersonId: string | null;
  /** Lead (has reports) links to their team roster instead of their own page. */
  leadsToTeam: boolean;
  filter: OrgTreeFilter | null;
  expansionOverrides: ReadonlyMap<string, boolean>;
  onExpandedChange: (personId: string, expanded: boolean) => void;
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
  const defaultOpen = depth === 0 || isActive || hasActiveDescendant;
  const open = filter
    ? true
    : (expansionOverrides.get(node.person_id) ?? defaultOpen);
  const label = personName(node) ?? UNNAMED_PERSON;
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
        <div
          className="flex min-w-0 items-center gap-1"
          style={{ paddingLeft: `${0.5 + depth * 0.875}rem` }}
        >
          {hasReports ? (
            filter ? (
              <span
                aria-hidden
                className="flex size-8 shrink-0 items-center justify-center text-muted-foreground [&>svg]:size-4"
              >
                <ChevronDown />
              </span>
            ) : (
              <SidebarMenuButton
                type="button"
                aria-expanded={open}
                aria-label={`${open ? "Collapse" : "Expand"} ${label}`}
                className="w-8 shrink-0 justify-center p-0 text-muted-foreground"
                onClick={() => onExpandedChange(node.person_id, !open)}
              >
                {open ? <ChevronDown /> : <ChevronRight />}
              </SidebarMenuButton>
            )
          ) : (
            <span aria-hidden className="size-8 shrink-0" />
          )}
          <SidebarMenuButton
            isActive={isActive}
            render={link}
            className="min-w-0 flex-1 ps-2"
          >
            {hasReports ? <Users /> : <User />}
            <span className="truncate">{label}</span>
          </SidebarMenuButton>
        </div>
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
              expansionOverrides={expansionOverrides}
              onExpandedChange={onExpandedChange}
            />
          ))
        : null}
    </>
  );
}

/**
 * The same navigation for an organisation with no reporting lines: one flat
 * list, in label order, scrolling in the pane exactly as the chart does. There
 * is no chart to draw and no depth to indent, so a row carries no chevron.
 */
function RosterList({
  query,
  roster,
}: {
  query: string;
  roster: readonly PeopleListItem[];
}) {
  const { personId: viewerPersonId } = useViewer();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const activePersonId = useMemo(() => {
    const fromPath = personIdFromPath(pathname);
    if (fromPath) return fromPath;
    if (pathname === "/" && viewerPersonId) return viewerPersonId;
    return null;
  }, [pathname, viewerPersonId]);

  const listed = useMemo(() => {
    const term = query.trim().toLowerCase();
    const rows = roster
      .map((person) => ({
        person,
        label: personName(person) ?? UNNAMED_PERSON,
      }))
      .sort((left, right) => left.label.localeCompare(right.label));
    return term
      ? rows.filter((row) => row.label.toLowerCase().includes(term))
      : rows;
  }, [roster, query]);

  if (!listed.length) {
    return (
      <p className="px-4 py-2 text-sm text-muted-foreground">
        No one here matches “{query.trim()}”
      </p>
    );
  }

  return (
    // Padding INSIDE the scroll region the pane owns, so the first and last
    // names clear its edges instead of touching them.
    <SidebarMenu className="pb-2">
      {listed.map(({ person, label }) => (
        <SidebarMenuItem key={person.person_id}>
          <SidebarMenuButton
            isActive={
              activePersonId
                ? personIdEq(activePersonId, person.person_id)
                : false
            }
            render={
              <Link
                to="/ic/$person/personal"
                params={{ person: person.person_id }}
              />
            }
          >
            <span className="w-4 shrink-0" />
            <User />
            <span className="truncate">{label}</span>
          </SidebarMenuButton>
        </SidebarMenuItem>
      ))}
    </SidebarMenu>
  );
}

/** Recursive org-chart navigation, rooted at the viewer. */
export function OrgTree({
  leadsToTeam = false,
  query = "",
}: { leadsToTeam?: boolean; query?: string } = {}) {
  const { isFlat } = useVisibilityPolicy();
  const { personId: viewerPersonId } = useViewer();
  const { roster } = useVisibleRoster(true);
  const viewer = useMemo(
    () => (viewerPersonId ? rosterTree(roster, viewerPersonId) : null),
    [roster, viewerPersonId]
  );
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const activePersonId = useMemo(() => {
    const fromPath = personIdFromPath(pathname);
    if (fromPath) return fromPath;
    if (pathname === "/" && viewerPersonId) return viewerPersonId;
    return null;
  }, [pathname, viewerPersonId]);
  const filter = useMemo(() => filterOrgTree(viewer, query), [viewer, query]);
  const [expansionOverrides, setExpansionOverrides] = useState<
    ReadonlyMap<string, boolean>
  >(() => new Map());

  const setExpanded = (personId: string, expanded: boolean) => {
    setExpansionOverrides((current) => {
      const next = new Map(current);
      next.set(personId, expanded);
      return next;
    });
  };

  if (isFlat) return <RosterList query={query} roster={roster} />;
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
        expansionOverrides={expansionOverrides}
        onExpandedChange={setExpanded}
      />
    </SidebarMenu>
  );
}
