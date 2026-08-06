import { Settings2 } from "lucide-react";

import { AppSidebarFooter } from "@/components/app-sidebar-footer";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import { type Zone } from "@/lib/portal/nav-model";
import { useShellLayout } from "@/lib/portal/use-shell-layout";
import { useZoneNav } from "@/lib/portal/use-zone-nav";

/**
 * Portal primary rail: a bounded set of zone icons. Entity zones (Person /
 * People) link to the existing dashboard routes and clear the theme-zone
 * selection; other zones set the active zone so the context pane switches.
 * Zones the active role can't see are filtered out (permission layer — FE
 * stub over the future role_section_visibility entity). Rendered as a
 * `collapsible="none"` sidebar so it sits in normal flow beside the pane.
 *
 * Below 768px the rail renders nothing: 56px of icons plus a 256px pane left a
 * phone with ~60px of content. The same zones (labelled, not icon-only) live in
 * the context pane's drawer instead — see `ContextPane`. On a tablet the rail
 * stays: 56px is affordable, and it is the pane that collapses.
 */
export function LensRail() {
  const layout = useShellLayout();
  const { zones, activeZone, selectZone } = useZoneNav();

  if (layout === "phone") return null;

  return (
    <Sidebar collapsible="none" className="w-14! border-e">
      <SidebarHeader className="items-center">
        <div className="flex size-8 items-center justify-center rounded-md bg-sidebar-primary text-sm font-bold text-sidebar-primary-foreground">
          I
        </div>
      </SidebarHeader>
      <SidebarContent>
        <SidebarMenu className="items-center gap-1">
          {zones.map((z) => (
            <ZoneItem
              key={z.id}
              zone={z}
              active={activeZone === z.id}
              onSelect={selectZone}
            />
          ))}
        </SidebarMenu>
      </SidebarContent>
      <SidebarFooter className="items-center gap-1">
        <Popover>
          <PopoverTrigger
            render={
              <button
                type="button"
                title="Settings"
                className="flex size-10 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
              >
                <Settings2 className="size-[19px]" aria-hidden />
                <span className="sr-only">Settings</span>
              </button>
            }
          />
          <PopoverContent side="right" align="end" className="w-56 gap-0 p-1">
            <AppSidebarFooter />
          </PopoverContent>
        </Popover>
      </SidebarFooter>
    </Sidebar>
  );
}

function ZoneItem({
  zone,
  active,
  onSelect,
}: {
  zone: Zone;
  active: boolean;
  onSelect: (zone: Zone) => void;
}) {
  const Icon = zone.icon;

  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        isActive={active}
        title={zone.label}
        className="size-10 justify-center p-0"
        onClick={() => onSelect(zone)}
      >
        <Icon />
        <span className="sr-only">{zone.label}</span>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}
