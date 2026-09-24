import { Link, useRouterState } from "@tanstack/react-router";
import {
  Database,
  LayoutDashboard,
  LayoutGrid,
  Share2,
  Sigma,
  Sparkles,
  Star,
  User,
  type LucideIcon,
} from "lucide-react";
import { useQuery } from "@tanstack/react-query";

import { CountBadge, UnbuiltRow } from "@/components/portal/pane-nav";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import type { PaneItem } from "@/lib/portal/nav-model";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { dashboardNamesQuery } from "@/queries/custom";

const PLANNED_BROWSE: readonly PaneItem[] = [
  { id: "mine", label: "My dashboards", icon: User, unbuilt: true },
  { id: "shared", label: "Shared with me", icon: Share2, unbuilt: true },
  { id: "starred", label: "Starred", icon: Star, unbuilt: true },
  { id: "starter", label: "Starter", icon: Sparkles, unbuilt: true },
];

const CATALOGUES: readonly {
  to: "/portal/custom/metrics" | "/portal/custom/widgets" | "/portal/custom/datasets";
  label: string;
  icon: LucideIcon;
}[] = [
  { to: "/portal/custom/metrics", label: "Metrics", icon: Sigma },
  { to: "/portal/custom/widgets", label: "Widgets", icon: LayoutDashboard },
  { to: "/portal/custom/datasets", label: "Datasets", icon: Database },
];

const NOT_A_DASHBOARD = new Set(["metrics", "widgets", "datasets", "new", "edit"]);

function onDashboards(pathname: string): boolean {
  const rest = pathname.replace(/\/$/, "").split("/").slice(3);
  return rest.length === 0 || (rest.length === 1 && !NOT_A_DASHBOARD.has(rest[0]!));
}

export function CustomNav() {
  const showPlanned = usePortalShowPlanned();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const { data } = useQuery(dashboardNamesQuery());

  return (
    <>
      <SidebarGroup>
        <SidebarGroupLabel>Browse</SidebarGroupLabel>
        <SidebarGroupContent>
          <SidebarMenu>
            <SidebarMenuItem>
              <SidebarMenuButton
                isActive={onDashboards(pathname)}
                render={<Link to="/portal/custom" />}
              >
                <LayoutGrid />
                <span>All dashboards</span>
                {data ? <CountBadge>{data.total}</CountBadge> : null}
              </SidebarMenuButton>
            </SidebarMenuItem>
            {showPlanned
              ? PLANNED_BROWSE.map((item) => <UnbuiltRow key={item.id} item={item} />)
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
