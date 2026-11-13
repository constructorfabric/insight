import { Link } from "@tanstack/react-router";
import { Bug, Megaphone, type LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";

import { useViewer } from "@/auth";
import { useFeedbackDialog } from "@/components/feedback-context";
import { personName } from "@/lib/identities/person-display";
import { SidebarSettings } from "@/components/sidebar-settings";
import { ThemeSwitcher } from "@/components/theme-switcher";
import { Avatar, AvatarFallback } from "@gears-frontx/ui-kit";
import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import { getInitials } from "@/lib/insight/get-initials";
import { resolveZoneItem } from "@/lib/portal/nav-model";
import { linkTo } from "@/lib/portal/portal-routes";
import { useNavLocation } from "@/lib/portal/use-nav-location";
import { useIcPerson } from "@/queries/ic-dashboard";

/**
 * Shared footer for the sidebar chrome: What's new, view
 * settings (planned sections), theme switch, and the viewer
 * identity block. The portal's rail and its context pane both mount it, so the
 * settings popover and the pane offer the same controls.
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
          surface="whats-new"
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

/** Active state resolves through `resolveZoneItem`, as the context pane's does. */
function MenuEntry({
  surface,
  icon: Icon,
  label,
  onNavigate,
}: {
  surface: string;
  icon: LucideIcon;
  label: string;
  onNavigate?: () => void;
}) {
  const { zone, item: picked } = useNavLocation();
  const item = resolveZoneItem(zone, picked);

  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        isActive={zone === "manage" && item === surface}
        render={
          <Link {...linkTo({ zone: "manage", item: surface })} onClick={onNavigate} />
        }
      >
        <Icon />
        <span>{label}</span>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}
