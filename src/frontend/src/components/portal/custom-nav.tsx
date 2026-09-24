import { Link, useRouterState } from "@tanstack/react-router";
import {
  Database,
  LayoutDashboard,
  LayoutGrid,
  Sigma,
  type LucideIcon,
} from "lucide-react";
import { useInfiniteQuery } from "@tanstack/react-query";

import { CountBadge, UnbuiltRow } from "@/components/portal/pane-nav";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import { DASHBOARD_BROWSE_PLANNED } from "@/lib/portal/nav-model";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { definitionPagesQuery } from "@/queries/custom";

const CATALOGUES: readonly {
  to: "/portal/custom/metrics" | "/portal/custom/widgets" | "/portal/custom/datasets";
  label: string;
  icon: LucideIcon;
}[] = [
  { to: "/portal/custom/metrics", label: "Metrics", icon: Sigma },
  { to: "/portal/custom/widgets", label: "Widgets", icon: LayoutDashboard },
  { to: "/portal/custom/datasets", label: "Datasets", icon: Database },
];

const DASHBOARD_ROUTES = new Set(["/portal/custom/", "/portal/custom/$name"]);

export function CustomNav() {
  const showPlanned = usePortalShowPlanned();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const routeId = useRouterState({ select: (s) => s.matches.at(-1)?.routeId });
  const { data } = useInfiniteQuery(definitionPagesQuery("dashboards"));
  const total = data?.pages[0]?.total;

  return (
    <>
      <SidebarGroup>
        <SidebarGroupLabel>Browse</SidebarGroupLabel>
        <SidebarGroupContent>
          <SidebarMenu>
            <SidebarMenuItem>
              <SidebarMenuButton
                isActive={routeId != null && DASHBOARD_ROUTES.has(routeId)}
                render={<Link to="/portal/custom" />}
              >
                <LayoutGrid />
                <span>All dashboards</span>
              </SidebarMenuButton>
              {total != null ? <CountBadge>{total}</CountBadge> : null}
            </SidebarMenuItem>
            {showPlanned
              ? DASHBOARD_BROWSE_PLANNED.map((item) => (
                  <UnbuiltRow key={item.id} item={item} />
                ))
              : null}
          </SidebarMenu>
        </SidebarGroupContent>
      </SidebarGroup>
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
    </>
  );
}
