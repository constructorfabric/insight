import { Link, useRouterState } from "@tanstack/react-router";
import { BookOpenText, Megaphone, type LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";

import { useViewer } from "@/auth";
import { SidebarSettings } from "@/components/sidebar-settings";
import { ThemeSwitcher } from "@/components/theme-switcher";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import { getInitials } from "@/lib/insight/get-initials";
import { resolveZoneItem } from "@/lib/portal/nav-model";
import { usePortalItem } from "@/lib/portal/portal-nav";
import { usePortalEnabled } from "@/lib/portal/portal-store";
import { useActiveZone } from "@/lib/portal/use-active-zone";
import { useIcPerson } from "@/queries/ic-dashboard";

/**
 * Shared footer for the sidebar chrome: metric catalog, What's new, view
 * settings (portal / focus / explanations), theme switch, and the viewer
 * identity block. Extracted from AppSidebar so the portal shell can surface
 * the same controls (from the rail's settings popover) without duplicating them.
 *
 * The first two entries follow the Portal toggle: the portal's Manage surfaces
 * while it is on, the standalone `/metrics` and `/whats-new` screens while it is
 * off. They used to name the standalone screens unconditionally, which dropped a
 * portal reader back into the previous interface. The toggle stays for now so
 * the old UI can still be looked at.
 *
 * `onNavigate` fires when one of those two is picked, carrying whether a
 * POINTER made the pick — the rail needs that to tell a pointer resting on it
 * from a keyboard visit. The portal mounts this in a popover, and a Manage
 * surface renders BEHIND that popover rather than replacing the shell it lives
 * in, so the menu has to be dismissed rather than destroyed. Whoever opened it
 * owns closing it; the footer only reports. The toggles below are deliberately
 * silent: flipping Portal or Focus is a setting, and a menu that shut on every
 * flip would need reopening each time.
 */
export function AppSidebarFooter({
  onNavigate,
}: {
  onNavigate?: (viaPointer: boolean) => void;
}) {
  const { t } = useTranslation();
  const { email: viewerEmail, personId: viewerPersonId } = useViewer();
  const viewerQ = useIcPerson(viewerPersonId ?? "");
  const viewer = viewerQ.data ?? null;

  const primaryEmail = viewer?.email ?? viewerEmail;
  const primary = viewer?.display_name || primaryEmail;
  const showSecondary = primary !== primaryEmail;

  return (
    <>
      <SidebarMenu>
        <ChromeEntry
          surface="metric-catalog"
          screen="/metrics"
          icon={BookOpenText}
          label={t("metric_definitions.nav_label")}
          onNavigate={onNavigate}
        />
        <ChromeEntry
          surface="whats-new"
          screen="/whats-new"
          icon={Megaphone}
          label={t("whats_new.nav_label")}
          onNavigate={onNavigate}
        />
      </SidebarMenu>
      <SidebarSettings />
      <ThemeSwitcher />
      {viewerEmail ? (
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton size="lg" className="cursor-default">
              <Avatar className="size-8 shrink-0">
                <AvatarFallback className="bg-sidebar-primary text-xs font-semibold text-sidebar-primary-foreground">
                  {getInitials(primary) || "?"}
                </AvatarFallback>
              </Avatar>
              <div className="flex min-w-0 flex-1 flex-col leading-tight">
                <span className="truncate text-sm font-medium text-sidebar-foreground">
                  {primary}
                </span>
                {showSecondary ? (
                  <span className="truncate text-xs text-sidebar-foreground/60">
                    {primaryEmail}
                  </span>
                ) : null}
              </div>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      ) : null}
    </>
  );
}

/**
 * One entry, pointing at its Manage surface in the portal.
 *
 * Active state comes from `resolveZoneItem` — the same resolver the context
 * pane and the zone content use — rather than a comparison of its own. A
 * private one disagreed with them twice: a bare `?zone=manage` marked nothing
 * here while the pane marked the zone's default, and a `?zone=` left behind on
 * a person route marked a Manage entry the portal was not showing.
 */
function ChromeEntry({
  surface,
  screen,
  icon: Icon,
  label,
  onNavigate,
}: {
  surface: string;
  screen: "/metrics" | "/whats-new";
  icon: LucideIcon;
  label: string;
  onNavigate?: (viaPointer: boolean) => void;
}) {
  const portal = usePortalEnabled();
  const { activeZone } = useActiveZone();
  const item = resolveZoneItem(activeZone, usePortalItem());
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  // `detail` counts pointer clicks; keyboard activation reports 0.
  const report = (e: { detail: number }) => onNavigate?.(e.detail > 0);

  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        isActive={
          portal
            ? activeZone === "manage" && item === surface
            : pathname === screen
        }
        render={
          portal ? (
            // `acct` belongs to the Identities console and means nothing on
            // another surface. Cleared rather than omitted: `retainSearchParams`
            // restores any portal key ABSENT from the target search.
            <Link
              to="/portal"
              search={{ zone: "manage", item: surface, acct: undefined }}
              onClick={report}
            />
          ) : (
            <Link to={screen} onClick={report} />
          )
        }
      >
        <Icon />
        <span>{label}</span>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}
