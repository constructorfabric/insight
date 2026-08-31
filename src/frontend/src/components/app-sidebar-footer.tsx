import { Link, useRouterState } from "@tanstack/react-router";
import { BookOpenText, Bug, Megaphone, type LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";

import { useViewer } from "@/auth";
import { useFeedbackDialog } from "@/components/feedback-context";
import { personName } from "@/lib/identities/person-display";
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
import { readLegacyShell } from "@/lib/portal/portal-store";
import { useActiveZone } from "@/lib/portal/use-active-zone";
import { useIcPerson } from "@/queries/ic-dashboard";

/**
 * Shared footer for the sidebar chrome: metric catalog, What's new, view
 * settings (portal / focus / explanations), theme switch, and the viewer
 * identity block. Extracted from AppSidebar so the portal shell can surface
 * the same controls (from the rail's settings popover) without duplicating them.
 *
 * `onNavigate` fires for the entries that put something else on screen: what
 * they open renders behind the popover the portal mounts this in, so the opener
 * has to dismiss it. The toggles stay silent — a menu that shut on every flip
 * would need reopening each time.
 */
export function AppSidebarFooter({
  onNavigate,
  showFeedback = true,
}: {
  onNavigate?: () => void;
  /** False where the shell already offers it — the rail has its own slot. */
  showFeedback?: boolean;
}) {
  const { t } = useTranslation();
  const feedback = useFeedbackDialog();
  const { email: viewerEmail, personId: viewerPersonId } = useViewer();
  const viewerQ = useIcPerson(viewerPersonId ?? "");
  const viewer = viewerQ.data ?? null;

  const primaryEmail = viewer?.email ?? viewerEmail;
  const primary = (viewer ? personName(viewer) : null) ?? primaryEmail;
  const showSecondary = primary !== primaryEmail;

  return (
    <>
      <SidebarMenu>
        <MenuEntry
          surface="metric-catalog"
          screen="/metrics"
          icon={BookOpenText}
          label={t("metric_definitions.nav_label")}
          onNavigate={onNavigate}
        />
        <MenuEntry
          surface="whats-new"
          screen="/whats-new"
          icon={Megaphone}
          label={t("whats_new.nav_label")}
          onNavigate={onNavigate}
        />
        {showFeedback && feedback ? (
          <SidebarMenuItem>
            <SidebarMenuButton
              onClick={() => {
                feedback.openFeedback();
                onNavigate?.();
              }}
            >
              <Bug />
              <span>{t("feedback.nav_label")}</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        ) : null}
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
 * Active state resolves through `resolveZoneItem`, the resolver the context
 * pane and the zone content use: a private comparison disagrees with them on a
 * bare `?zone=manage` and on a `?zone=` stranded by a person route.
 */
function MenuEntry({
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
  onNavigate?: () => void;
}) {
  const portal = !readLegacyShell();
  const { activeZone } = useActiveZone();
  const item = resolveZoneItem(activeZone, usePortalItem());
  const pathname = useRouterState({ select: (s) => s.location.pathname });

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
            // `acct` is cleared, not omitted: `retainSearchParams` restores
            // any portal key ABSENT from the target search.
            <Link
              to="/portal"
              search={{ zone: "manage", item: surface, acct: undefined }}
              onClick={onNavigate}
            />
          ) : (
            <Link to={screen} onClick={onNavigate} />
          )
        }
      >
        <Icon />
        <span>{label}</span>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}
