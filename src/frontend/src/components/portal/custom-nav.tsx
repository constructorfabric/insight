import { Link, useRouterState } from "@tanstack/react-router";
import {
  Bell,
  Database,
  LayoutDashboard,
  Sigma,
  type LucideIcon,
} from "lucide-react";

import { useDashboardBrowse } from "@/lib/custom/dashboard-browse";
import { FoldersGroup } from "@/components/portal/folder-rows";
import { CountBadge, UnbuiltRow } from "@/components/portal/pane-nav";
import { PinnedGroup } from "@/components/portal/pinned-rows";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import { DASHBOARD_BROWSE_PLANNED, STARTER_DASHBOARDS } from "@/lib/portal/nav-model";
import { usePortalSearch } from "@/lib/portal/portal-search";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { useIsAdmin } from "@/queries/identity-me";

const CATALOGUES: readonly {
  to:
    | "/portal/custom/metrics"
    | "/portal/custom/widgets"
    | "/portal/custom/datasets"
    | "/portal/custom/alerts";
  label: string;
  icon: LucideIcon;
}[] = [
  { to: "/portal/custom/metrics", label: "Metrics", icon: Sigma },
  { to: "/portal/custom/widgets", label: "Widgets", icon: LayoutDashboard },
  { to: "/portal/custom/datasets", label: "Datasets", icon: Database },
  { to: "/portal/custom/alerts", label: "Alerts", icon: Bell },
];

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
        <SidebarGroupLabel>Browse</SidebarGroupLabel>
        <SidebarGroupContent>
          <SidebarMenu>
            {browse.map(({ owned: narrowed, label, icon: Icon, count }) => (
              <SidebarMenuItem key={narrowed ?? "all"}>
                <SidebarMenuButton
                  isActive={onDashboards && !folder && owned === narrowed}
                  render={
                    <Link to="/portal/custom" search={{ owned: narrowed, tag }} />
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
      {isAdmin ? <CatalogueGroup pathname={pathname} /> : null}
      <PinnedGroup />
    </>
  );
}

function CatalogueGroup({ pathname }: { pathname: string }) {
  return (
    <SidebarGroup>
      <SidebarGroupLabel>Catalogue</SidebarGroupLabel>
      <SidebarGroupContent>
        <SidebarMenu>
          {CATALOGUES.map(({ to, label, icon: Icon }) => (
            <SidebarMenuItem key={to}>
              <SidebarMenuButton
                isActive={pathname.startsWith(to)}
                render={<Link to={to} />}
              >
                <Icon />
                <span>{label}</span>
              </SidebarMenuButton>
            </SidebarMenuItem>
          ))}
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
      <SidebarGroupLabel>Dashboards</SidebarGroupLabel>
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
              {mine.count != null ? <CountBadge>{mine.count}</CountBadge> : null}
            </SidebarMenuItem>
          ) : null}
          {showPlanned ? <UnbuiltRow item={STARTER_DASHBOARDS} /> : null}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}
