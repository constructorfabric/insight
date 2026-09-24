import { ChevronRight, MessageSquare, Settings2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { AppSidebarFooter } from "@/components/app-sidebar-footer";
import { useFeedbackDialog } from "@/components/feedback-context";
import { CustomNav } from "@/components/portal/custom-nav";
import { ExploreNav } from "@/components/portal/explore-nav";
import { GroupsNav } from "@/components/portal/pane-nav";
import { PeopleNav } from "@/components/portal/people-nav";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
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
import {
  HOME_DASHBOARD_GROUP,
  manageGroupsFor,
  resolveZoneItem,
  zoneSections,
} from "@/lib/portal/nav-model";
import { usePortalItem } from "@/lib/portal/portal-nav";
import { railItemFor } from "@/lib/portal/rail-model";
import { useActiveZone } from "@/lib/portal/use-active-zone";
import { useDismissDrawer } from "@/lib/portal/use-dismiss-drawer";
import { useRailNav } from "@/lib/portal/use-rail-nav";
import { useShellLayout } from "@/lib/portal/use-shell-layout";
import { useZoneNav } from "@/lib/portal/use-zone-nav";
import { cn } from "@/lib/utils";
import { useIsAdmin } from "@/queries/identity-me";
import { usePreviewsGate } from "@/queries/previews";

const HOME_TITLE = "Constructor Insight";

/**
 * Secondary navigation for the rail item that owns the active zone.
 *
 * On a phone this is the ONLY navigation surface: the icon rail hides itself
 * (two fixed sidebars left ~60px for content), so the pane becomes an
 * off-canvas drawer — opened by the topbar trigger — and carries the rail items
 * and the settings menu that normally live in the rail. Desktop is unchanged:
 * `collapsible="none"`, in normal flow, rail beside it.
 */
export function ContextPane() {
  const layout = useShellLayout();
  // A phone hides the rail, so the drawer inherits its duties. A tablet keeps
  // the rail — the drawer there is only the pane itself, collapsed to give the
  // content its 256px back.
  const isPhone = layout === "phone";
  const drawer = layout !== "wide";
  const { activeZone } = useActiveZone();
  const { zones, selectZone } = useZoneNav();
  const visibleZones = new Set(zones.map((zone) => zone.id));
  const rail = railItemFor(activeZone);
  const title = rail?.id === "home" ? HOME_TITLE : (rail?.label ?? "Insight");
  const active = resolveZoneItem(activeZone, usePortalItem());
  const [settingsOpen, setSettingsOpen] = useState(false);
  const dismissDrawer = useDismissDrawer();
  const personZone = zones.find((zone) => zone.id === "person");

  return (
    <Sidebar
      collapsible={drawer ? "offcanvas" : "none"}
      className={cn(
        "border-e",
        layout === "narrow" && "data-[side=left]:left-(--rail-width)"
      )}
    >
      {/* The drawer's rail row already names the pane, so repeating it in a
          header would cost two of the ~14 rows a phone has. */}
      {isPhone ? null : (
        <SidebarHeader>
          <span className="px-2 py-1.5 text-sm font-semibold tracking-tight text-sidebar-foreground">
            {title}
          </span>
        </SidebarHeader>
      )}
      <SidebarContent>
        {isPhone ? <MobileRailNav /> : null}
        {rail?.id === "home" ? (
          <GroupsNav
            groups={[
              ...zoneSections("overview"),
              ...(visibleZones.has("custom") ? [HOME_DASHBOARD_GROUP] : []),
            ]}
            active={active}
          />
        ) : rail?.id === "dashboards" ? (
          <CustomNav />
        ) : rail?.id === "explore" ? (
          <ExploreNav visibleZones={visibleZones} />
        ) : rail?.id === "people" ? (
          <PeopleNav
            visibleZones={visibleZones}
            onOpenMe={() => {
              if (personZone) selectZone(personZone);
            }}
          />
        ) : rail?.id === "reports" ? (
          <GroupsNav groups={zoneSections("reports")} active={active} />
        ) : rail?.id === "manage" ? (
          <ManageNav active={active} />
        ) : null}
      </SidebarContent>
      {isPhone ? (
        <SidebarFooter>
          <SidebarMenu>
            <FeedbackItem onPicked={dismissDrawer} />
            {/* One row, not six: inline the settings menu and it takes a third
                of the drawer, crowding out the sections that are the point of
                it. Same affordance the rail gives desktop — an icon that opens
                the menu on demand. */}
            <SidebarMenuItem>
              <Popover open={settingsOpen} onOpenChange={setSettingsOpen}>
                <PopoverTrigger
                  render={
                    <SidebarMenuButton>
                      <Settings2 aria-hidden />
                      <span>Settings</span>
                    </SidebarMenuButton>
                  }
                />
                <PopoverContent
                  side="top"
                  align="start"
                  className="w-60 gap-0 p-1"
                >
                  {/* A leaf pick, so it dismisses the drawer as every other
                      one here does — and the popover with it, which on a phone
                      covers the surface just asked for. */}
                  <AppSidebarFooter
                    showFeedback={false}
                    onNavigate={() => {
                      setSettingsOpen(false);
                      dismissDrawer();
                    }}
                  />
                </PopoverContent>
              </Popover>
            </SidebarMenuItem>
          </SidebarMenu>
        </SidebarFooter>
      ) : null}
    </Sidebar>
  );
}

/** Its own drawer row: inside the settings menu nobody found it. */
function FeedbackItem({ onPicked }: { onPicked: () => void }) {
  const { t } = useTranslation();
  const feedback = useFeedbackDialog();

  if (!feedback) return null;

  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        onClick={() => {
          feedback.openFeedback();
          onPicked();
        }}
      >
        <MessageSquare aria-hidden />
        <span>{t("feedback.nav_label")}</span>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}

/**
 * Rail switcher for the mobile drawer, standing in for the hidden icon rail.
 *
 * Collapsed to a SINGLE row by default — expanded, the full list pushed the
 * pane's own sections below the fold, so picking an item looked like it did
 * nothing. Collapsed, the sections start right under this row: changing section
 * (the common move) costs no scrolling, and switching rail item costs one extra
 * tap that also re-collapses the list.
 */
function MobileRailNav() {
  const { items, activeItem, selectItem } = useRailNav();
  const [expanded, setExpanded] = useState(false);
  const current = items.find((item) => item.id === activeItem);
  const CurrentIcon = current?.icon;

  return (
    <SidebarGroup>
      <SidebarGroupContent>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton
              onClick={() => setExpanded((v) => !v)}
              aria-expanded={expanded}
            >
              {CurrentIcon ? <CurrentIcon aria-hidden /> : null}
              <span className="font-medium">{current?.label ?? "Menu"}</span>
              <ChevronRight
                className={cn(
                  "ms-auto transition-transform",
                  expanded && "rotate-90"
                )}
                aria-hidden
              />
            </SidebarMenuButton>
          </SidebarMenuItem>
          {expanded
            ? items.map((item) => (
                <SidebarMenuItem key={item.id}>
                  <SidebarMenuButton
                    isActive={activeItem === item.id}
                    onClick={() => {
                      selectItem(item);
                      setExpanded(false);
                    }}
                    className={cn("ps-4", item.planned && "text-muted-foreground")}
                  >
                    <item.icon aria-hidden />
                    <span>{item.label}</span>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              ))
            : null}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}

function ManageNav({ active }: { active: string | null }) {
  // Gated surfaces (Identities, Previews) drop from the pane for everyone
  // else; the view behind each refuses direct URLs on its own.
  const { isAdmin } = useIsAdmin();
  const canManagePreviews = usePreviewsGate();
  return (
    <GroupsNav groups={manageGroupsFor({ isAdmin, canManagePreviews })} active={active} />
  );
}
