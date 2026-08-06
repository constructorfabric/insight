import { Link, useRouterState } from "@tanstack/react-router";
import { BookOpenText, Megaphone } from "lucide-react";
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
import { useIcPerson } from "@/queries/ic-dashboard";

/**
 * Shared footer for the sidebar chrome: metric catalog, What's new, view
 * settings (portal / focus / explanations), theme switch, and the viewer
 * identity block. Extracted from AppSidebar so the portal shell can surface
 * the same controls (from the rail's settings popover) without duplicating them.
 */
export function AppSidebarFooter() {
  const { t } = useTranslation();
  const { email: viewerEmail, personId: viewerPersonId } = useViewer();
  const viewerQ = useIcPerson(viewerPersonId ?? "");
  const viewer = viewerQ.data ?? null;
  const pathname = useRouterState({ select: (s) => s.location.pathname });

  const primaryEmail = viewer?.email ?? viewerEmail;
  const primary = viewer?.display_name || primaryEmail;
  const showSecondary = primary !== primaryEmail;

  return (
    <>
      <SidebarMenu>
        <SidebarMenuItem>
          <SidebarMenuButton
            isActive={pathname === "/metrics"}
            render={<Link to="/metrics" />}
          >
            <BookOpenText />
            <span>{t("metric_definitions.nav_label")}</span>
          </SidebarMenuButton>
        </SidebarMenuItem>
        <SidebarMenuItem>
          <SidebarMenuButton
            isActive={pathname === "/whats-new"}
            render={<Link to="/whats-new" />}
          >
            <Megaphone />
            <span>{t("whats_new.nav_label")}</span>
          </SidebarMenuButton>
        </SidebarMenuItem>
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
