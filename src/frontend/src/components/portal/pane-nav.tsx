import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { PaneGroupLabel } from "@/components/portal/pane-group-label";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import { resolveText, type DisplayText } from "@/i18n/localized-text";
import {
  PLANNED_GROUP_LABEL,
  partitionByReadiness,
  type PaneGroup,
  type PaneItem,
} from "@/lib/portal/nav-model";
import { usePortalNavActions } from "@/lib/portal/portal-nav";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { useDismissDrawer } from "@/lib/portal/use-dismiss-drawer";
import { cn } from "@/lib/utils";

const BADGE_TONE: Record<string, string> = {
  warn: "bg-warning/15 text-warning",
  new: "bg-primary/10 text-foreground",
  error: "bg-destructive/15 text-destructive",
};

export function CountBadge({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <SidebarMenuBadge
      className={cn(
        "rounded-full bg-muted px-1.5 font-normal text-muted-foreground",
        className
      )}
    >
      {children}
    </SidebarMenuBadge>
  );
}

export function ItemBadge({ item }: { item: PaneItem }) {
  if (!item.badge) return null;
  return (
    <span
      className={cn(
        "ms-auto rounded-full px-1.5 py-0.5 text-xs font-semibold",
        BADGE_TONE[item.badge.tone]
      )}
    >
      {item.badge.text}
    </span>
  );
}

function ItemLabel({ text }: { text: DisplayText }) {
  const { t } = useTranslation();
  return <span>{resolveText(t, text)}</span>;
}

export function UnbuiltRow({ item }: { item: PaneItem }) {
  const Icon = item.icon;
  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        data-planned
        aria-disabled
        className="text-muted-foreground aria-disabled:opacity-100"
      >
        <Icon />
        <ItemLabel text={item.label} />
      </SidebarMenuButton>
      <SidebarMenuBadge className="font-normal text-muted-foreground">
        {PLANNED_GROUP_LABEL}
      </SidebarMenuBadge>
    </SidebarMenuItem>
  );
}

export function ItemButton({
  item,
  active,
  onPick,
}: {
  item: PaneItem;
  active: boolean;
  onPick?: () => void;
}) {
  const { setItem } = usePortalNavActions();
  const dismiss = useDismissDrawer();
  if (item.unbuilt) return <UnbuiltRow item={item} />;

  const Icon = item.icon;
  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        data-planned={item.readiness != null || undefined}
        isActive={active}
        onClick={() => {
          if (onPick) onPick();
          else setItem(item.id);
          dismiss();
        }}
        className={item.readiness != null ? "text-muted-foreground" : undefined}
      >
        <Icon />
        <ItemLabel text={item.label} />
        <ItemBadge item={item} />
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}

export function GroupsNav({
  groups,
  active,
}: {
  groups: readonly PaneGroup[];
  active: string | null;
}) {
  const showPlanned = usePortalShowPlanned();
  const split = groups.map((g) => partitionByReadiness(g.items, showPlanned));
  const planned = split.flatMap((s) => s.planned);

  return (
    <>
      {groups.map((g, i) =>
        split[i]!.live.length ? (
          <SidebarGroup key={g.label ?? i}>
            {g.label ? <PaneGroupLabel>{g.label}</PaneGroupLabel> : null}
            <SidebarGroupContent>
              <SidebarMenu>
                {split[i]!.live.map((it) => (
                  <ItemButton key={it.id} item={it} active={active === it.id} />
                ))}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        ) : null
      )}
      {planned.length ? (
        <SidebarGroup>
          <PaneGroupLabel>{PLANNED_GROUP_LABEL}</PaneGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {planned.map((it) => (
                <ItemButton key={it.id} item={it} active={active === it.id} />
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      ) : null}
    </>
  );
}
