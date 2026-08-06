import { Link, useRouterState } from "@tanstack/react-router";
import {
  ChevronDown,
  ChevronRight,
  User,
  Users,
} from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { useViewer } from "@/auth";
import { AppSidebarFooter } from "@/components/app-sidebar-footer";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import { personIdFromPath } from "@/lib/metrics/entity";
import { useIcPerson } from "@/queries/ic-dashboard";
import type { IdentityPerson } from "@/types/insight";

// The identity contract admits people with no email and no display name (a
// person whose log carries neither). Their node still has to be clickable.
const UNNAMED_PERSON = "Unnamed person";

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
}: {
  node: IdentityPerson;
  depth: number;
  activePersonId: string | null;
}) {
  const hasReports = node.subordinates.length > 0;
  const isActive = activePersonId
    ? personIdEq(activePersonId, node.person_id)
    : false;
  const hasActiveDescendant =
    hasReports && activePersonId
      ? node.subordinates.some((s) => containsPerson(s, activePersonId))
      : false;
  const open = depth === 0 || isActive || hasActiveDescendant;
  return (
    <>
      <SidebarMenuItem>
        <SidebarMenuButton
          isActive={isActive}
          render={
            <Link
              to="/ic/$person/personal"
              params={{ person: node.person_id }}
            />
          }
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
          <span className="truncate">
            {node.display_name || node.email || UNNAMED_PERSON}
          </span>
        </SidebarMenuButton>
      </SidebarMenuItem>
      {hasReports && open
        ? node.subordinates.map((sub) => (
            <PersonNode
              key={sub.person_id}
              node={sub}
              depth={depth + 1}
              activePersonId={activePersonId}
            />
          ))
        : null}
    </>
  );
}

export function AppSidebar() {
  const { t } = useTranslation();
  const { personId: viewerPersonId } = useViewer();
  const viewerQ = useIcPerson(viewerPersonId ?? "");
  const viewer = viewerQ.data ?? null;
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  // The URL segment is the person id since the identity cutover; a legacy
  // email URL simply highlights nothing for the moment its redirect takes.
  const activePersonId = useMemo(() => {
    const fromPath = personIdFromPath(pathname);
    if (fromPath) return fromPath;
    if (pathname === "/" && viewerPersonId) return viewerPersonId;
    return null;
  }, [pathname, viewerPersonId]);

  return (
    <Sidebar>
      <SidebarHeader>
        <div className="flex items-center gap-2 px-2 py-1.5">
          <div className="flex size-7 items-center justify-center rounded-md bg-sidebar-primary font-semibold text-sidebar-primary-foreground">
            I
          </div>
          <span className="font-semibold tracking-tight text-sidebar-foreground">
            {t("common.app_name")}
          </span>
        </div>
      </SidebarHeader>
      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupContent>
            {viewer ? (
              <SidebarMenu>
                <PersonNode node={viewer} depth={0} activePersonId={activePersonId} />
              </SidebarMenu>
            ) : null}
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
      <SidebarFooter>
        <AppSidebarFooter />
      </SidebarFooter>
    </Sidebar>
  );
}
