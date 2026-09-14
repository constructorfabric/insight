import { Link, useRouterState } from "@tanstack/react-router";
import { ChartLine, LayoutDashboard, Sigma } from "lucide-react";
import { useQuery } from "@tanstack/react-query";

import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";
import { dashboardNamesQuery, dashboardQuery } from "@/queries/custom";
import { TEXT_LABEL } from "@/lib/type-scale";

/**
 * The custom zone's pane: the three catalogues, then one row per dashboard
 * read from the same query the page reads — so a dashboard the chat just
 * built appears here as soon as the list is invalidated, with no reload.
 */
export function CustomNav() {
  return (
    <>
      <SidebarGroup>
        <SidebarGroupLabel>Catalogue</SidebarGroupLabel>
        <SidebarGroupContent>
          <SidebarMenu>
            <CatalogueRow
              to="/portal/custom"
              label="Dashboards"
              icon={<LayoutDashboard />}
              exact
            />
            <CatalogueRow
              to="/portal/custom/metrics"
              label="Metrics"
              icon={<Sigma />}
            />
            <CatalogueRow
              to="/portal/custom/widgets"
              label="Widgets"
              icon={<ChartLine />}
            />
          </SidebarMenu>
        </SidebarGroupContent>
      </SidebarGroup>
      <DashboardRows />
    </>
  );
}

function CatalogueRow({
  to,
  label,
  icon,
  exact = false,
}: {
  to: "/portal/custom" | "/portal/custom/metrics" | "/portal/custom/widgets";
  label: string;
  icon: React.ReactNode;
  /** Dashboards owns the zone root, so it must not match every child path. */
  exact?: boolean;
}) {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const active = exact ? pathname === to : pathname.startsWith(to);

  return (
    <SidebarMenuItem>
      <SidebarMenuButton isActive={active} render={<Link to={to} />}>
        {icon}
        <span>{label}</span>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}

/**
 * One row, labelled by the dashboard's own title rather than the identifier it
 * is stored under. The listing carries names only, so each row reads its own
 * definition - the same query the page reads, so opening one costs nothing
 * extra, and the name stands in until the title arrives.
 */
function DashboardRow({ name, active }: { name: string; active: boolean }) {
  const { data } = useQuery(dashboardQuery(name));

  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        isActive={active}
        render={<Link to="/portal/custom/$name" params={{ name }} />}
      >
        <LayoutDashboard />
        <span>{data?.title ?? name}</span>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}

function DashboardRows() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const { data, isPending, isError } = useQuery(dashboardNamesQuery());
  const names = data?.names;

  return (
    <SidebarGroup>
      <SidebarGroupLabel>Dashboards</SidebarGroupLabel>
      <SidebarGroupContent>
        {isPending ? (
          <div className="px-2 py-1.5">
            <Spinner className="size-4" />
          </div>
        ) : isError ? (
          <p className={cn(TEXT_LABEL, "px-2 py-1.5")}>
            Couldn&apos;t load dashboards.
          </p>
        ) : names && names.length > 0 ? (
          <SidebarMenu>
            {names.map((name) => (
              <DashboardRow
                key={name}
                name={name}
                active={pathname === `/portal/custom/${name}`}
              />
            ))}
          </SidebarMenu>
        ) : (
          <p className={cn(TEXT_LABEL, "px-2 py-1.5")}>
            No dashboards yet. Ask the assistant to build one.
          </p>
        )}
      </SidebarGroupContent>
    </SidebarGroup>
  );
}
