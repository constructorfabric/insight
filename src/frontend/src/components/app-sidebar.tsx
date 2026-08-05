import { Link, useRouterState } from "@tanstack/react-router";
import {
  BookOpenText,
  ChevronDown,
  ChevronRight,
  Megaphone,
  User,
  Users,
} from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { useViewer } from "@/auth";
import { SidebarSettings } from "@/components/sidebar-settings";
import { ThemeSwitcher } from "@/components/theme-switcher";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
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
import { getInitials } from "@/lib/insight/get-initials";
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
  const { email: viewerEmail, personId: viewerPersonId } = useViewer();
  const viewerQ = useIcPerson(viewerPersonId ?? "");
  const viewer = viewerQ.data ?? null;
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  // The URL segment is the person id since the identity cutover; a legacy
  // email URL simply highlights nothing for the moment its redirect takes.
  const activePersonId = useMemo(() => {
    const m = /^\/ic\/([^/]+)/.exec(pathname);
    if (m) return decodeURIComponent(m[1]!);
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
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton
              isActive={pathname === "/metrics"}
              render={<Link to="/metrics" />}
            >
              <BookOpenText />
              <span>{t("metric_definitions.nav_label")}</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
          <SidebarMenuItem>
            <SidebarMenuButton
              isActive={pathname === "/whats-new"}
              render={<Link to="/whats-new" />}
            >
              <Megaphone />
              <span>{t("whats_new.nav_label")}</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
        <SidebarSettings />
        <ThemeSwitcher />
        {viewerEmail
          ? (() => {
              const primaryEmail = viewer?.email ?? viewerEmail;
              const primary = viewer?.display_name || primaryEmail;
              const showSecondary = primary !== primaryEmail;
              return (
                <SidebarMenu>
                  <SidebarMenuItem>
                    <SidebarMenuButton size="lg" className="cursor-default">
                      <Avatar className="size-8 shrink-0">
                        <AvatarFallback className="bg-sidebar-primary text-xs font-semibold text-sidebar-primary-foreground">
                          {getInitials(primary) || "?"}
                        </AvatarFallback>
                      </Avatar>
                      <div className="flex min-w-0 flex-1 flex-col leading-tight">
                        <span className="truncate text-sm font-medium text-sidebar-foreground">
                          {primary}
                        </span>
                        {showSecondary ? (
                          <span className="truncate text-xs text-sidebar-foreground/60">
                            {primaryEmail}
                          </span>
                        ) : null}
                      </div>
                    </SidebarMenuButton>
                  </SidebarMenuItem>
                </SidebarMenu>
              );
            })()
          : null}
      </SidebarFooter>
    </Sidebar>
  );
}
