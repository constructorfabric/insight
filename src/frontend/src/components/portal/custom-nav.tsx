import { Link, useRouterState } from "@tanstack/react-router";

import { visibleCatalogues } from "@/lib/custom/catalogues";
import { useDashboardBrowse } from "@/lib/custom/dashboard-browse";
import { FoldersGroup } from "@/components/portal/folder-rows";
import { CountBadge, UnbuiltRow } from "@/components/portal/pane-nav";
import { PinnedGroup } from "@/components/portal/pinned-rows";
import { PaneGroupLabel } from "@/components/portal/pane-group-label";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import {
  DASHBOARD_BROWSE_PLANNED,
  STARTER_DASHBOARDS,
} from "@/lib/portal/nav-model";
import { usePortalSearch } from "@/lib/portal/portal-search";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { useIsAdmin } from "@/queries/identity-me";

const DASHBOARD_ROUTES = new Set(["/portal/custom/", "/portal/custom/$name"]);

export function CustomNav() {
  const showPlanned = usePortalShowPlanned();
  const { isAdmin } = useIsAdmin();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const routeId = useRouterState({ select: (s) => s.matches.at(-1)?.routeId });
  const { folder, tag, owned } = usePortalSearch();
  const browse = useDashboardBrowse(tag);
  const onDashboards = routeId != null && DASHBOARD_ROUTES.has(routeId);

  return (
    <>
      <SidebarGroup>
        <PaneGroupLabel>Browse</PaneGroupLabel>
        <SidebarGroupContent>
          <SidebarMenu>
            {browse.map(({ owned: narrowed, label, icon: Icon, count }) => (
              <SidebarMenuItem key={narrowed ?? "all"}>
                <SidebarMenuButton
                  isActive={onDashboards && !folder && owned === narrowed}
                  render={
                    <Link
                      to="/portal/custom"
                      search={{ owned: narrowed, tag }}
                    />
                  }
                >
                  <Icon />
                  <span>{label}</span>
                </SidebarMenuButton>
                {count != null ? <CountBadge>{count}</CountBadge> : null}
              </SidebarMenuItem>
            ))}
            {showPlanned
              ? DASHBOARD_BROWSE_PLANNED.map((item) => (
                  <UnbuiltRow key={item.id} item={item} />
                ))
              : null}
          </SidebarMenu>
        </SidebarGroupContent>
      </SidebarGroup>
      <FoldersGroup />
      {isAdmin ? (
        <CatalogueGroup pathname={pathname} showPlanned={showPlanned} />
      ) : null}
      <PinnedGroup />
    </>
  );
}

function CatalogueGroup({
  pathname,
  showPlanned,
}: {
  pathname: string;
  showPlanned: boolean;
}) {
  return (
    <SidebarGroup>
      <PaneGroupLabel>Catalogue</PaneGroupLabel>
      <SidebarGroupContent>
        <SidebarMenu>
          {visibleCatalogues(showPlanned).map(
            ({ to, label, icon: Icon, planned }) => (
              <SidebarMenuItem key={to}>
                <SidebarMenuButton
                  data-planned={planned || undefined}
                  isActive={pathname.startsWith(to)}
                  render={<Link to={to} />}
                  className={planned ? "text-muted-foreground" : undefined}
                >
                  <Icon />
                  <span>{label}</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
            )
          )}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}

export function HomeDashboardsGroup() {
  const showPlanned = usePortalShowPlanned();
  const mine = useDashboardBrowse().find((row) => row.owned === "me");

  if (!mine && !showPlanned) return null;

  return (
    <SidebarGroup>
      <PaneGroupLabel>Dashboards</PaneGroupLabel>
      <SidebarGroupContent>
        <SidebarMenu>
          {mine ? (
            <SidebarMenuItem>
              <SidebarMenuButton
                render={<Link to="/portal/custom" search={{ owned: "me" }} />}
              >
                <mine.icon />
                <span>{mine.label}</span>
              </SidebarMenuButton>
              {mine.count != null ? (
                <CountBadge>{mine.count}</CountBadge>
              ) : null}
            </SidebarMenuItem>
          ) : null}
          {showPlanned ? <UnbuiltRow item={STARTER_DASHBOARDS} /> : null}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}
