import { Link, useRouterState } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { Pin } from "lucide-react";

import type { Listed } from "@/api/custom-client";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import { dashboardIdFromPath } from "@/lib/custom/dashboard-path";
import { dashboardQuery, pinsQuery } from "@/queries/custom";

export function PinnedGroup() {
  const { data: pins } = useQuery(pinsQuery());
  const open = useRouterState({
    select: (s) => dashboardIdFromPath(s.location.pathname),
  });

  if (!pins || pins.length === 0) return null;

  return (
    <SidebarGroup>
      <SidebarGroupLabel>Pinned</SidebarGroupLabel>
      <SidebarGroupContent>
        <SidebarMenu>
          {pins.map((pin) => (
            <PinnedRow key={pin.id} pin={pin} active={pin.id === open} />
          ))}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}

function PinnedRow({ pin, active }: { pin: Listed; active: boolean }) {
  const { data } = useQuery(dashboardQuery(pin.id));

  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        isActive={active}
        render={<Link to="/portal/custom/$name" params={{ name: pin.id }} />}
      >
        <Pin />
        <span>{data?.title || pin.name}</span>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}
