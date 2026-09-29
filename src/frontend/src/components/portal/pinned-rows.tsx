import { Link, useRouterState } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { Pin } from "lucide-react";

import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import { dashboardNameFromPath } from "@/lib/custom/dashboard-path";
import { dashboardQuery, pinsQuery } from "@/queries/custom";

export function PinnedGroup() {
  const { data: pins } = useQuery(pinsQuery());
  const open = useRouterState({
    select: (s) => dashboardNameFromPath(s.location.pathname),
  });

  if (!pins || pins.length === 0) return null;

  return (
    <SidebarGroup>
      <SidebarGroupLabel>Pinned</SidebarGroupLabel>
      <SidebarGroupContent>
        <SidebarMenu>
          {pins.map((name) => (
            <PinnedRow key={name} name={name} active={name === open} />
          ))}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}

function PinnedRow({ name, active }: { name: string; active: boolean }) {
  const { data } = useQuery(dashboardQuery(name));

  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        isActive={active}
        render={<Link to="/portal/custom/$name" params={{ name }} />}
      >
        <Pin />
        <span>{data?.title || name}</span>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}
