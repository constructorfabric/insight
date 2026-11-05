import { Wrench } from "lucide-react";

import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import { Switch } from "@gears-frontx/ui-kit";
import {
  setPortalShowPlanned,
  usePortalShowPlanned,
} from "@/lib/portal/portal-store";

export function SidebarSettings() {
  const showPlanned = usePortalShowPlanned();

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <SidebarMenuButton
          onClick={() => setPortalShowPlanned(!showPlanned)}
          aria-pressed={showPlanned}
          className="justify-between"
        >
          <span className="flex items-center gap-2">
            <Wrench className="size-4" />
            <span>Show planned sections</span>
          </span>
          <Switch
            checked={showPlanned}
            onCheckedChange={setPortalShowPlanned}
            size="sm"
            tabIndex={-1}
          />
        </SidebarMenuButton>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}
