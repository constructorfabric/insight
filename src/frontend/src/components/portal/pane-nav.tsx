import type { ReactNode } from "react";

import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
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

export function CountBadge({ children }: { children: ReactNode }) {
  return (
    <span className="ms-auto rounded-full bg-muted px-1.5 text-xs text-muted-foreground tabular-nums">
      {children}
    </span>
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

export function UnbuiltRow({ item }: { item: PaneItem }) {
  const Icon = item.icon;
  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        aria-disabled
        className="text-muted-foreground aria-disabled:opacity-100"
      >
        <Icon />
        <span>{item.label}</span>
        <span className="ms-auto text-xs">{PLANNED_GROUP_LABEL}</span>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}

export function ItemButton({
  item,
  active,
  planned = false,
  onPick,
}: {
  item: PaneItem;
  active: boolean;
  /** Demoted rendering: same affordance, visibly lighter weight. */
  planned?: boolean;
  onPick?: () => void;
}) {
  const { setItem } = usePortalNavActions();
  const dismiss = useDismissDrawer();
  if (item.unbuilt) return <UnbuiltRow item={item} />;

  const Icon = item.icon;
  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        isActive={active}
        onClick={() => {
          if (onPick) onPick();
          else setItem(item.id);
          dismiss();
        }}
        className={planned ? "text-muted-foreground" : undefined}
      >
        <Icon />
        <span>{item.label}</span>
        <ItemBadge item={item} />
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}

// INVARIANT: an `unbuilt` row opens nothing and stays in its group; an
// install-planned entry still opens and moves to the demoted group below.
export function GroupsNav({
  groups,
  active,
}: {
  groups: readonly PaneGroup[];
  active: string | null;
}) {
  const showPlanned = usePortalShowPlanned();
  const split = groups.map((g) =>
    partitionByReadiness(
      g.items.filter((item) => !item.unbuilt || showPlanned),
      showPlanned,
    ),
  );
  const planned = split.flatMap((s) => s.planned);

  return (
    <>
      {groups.map((g, i) =>
        split[i]!.live.length ? (
          <SidebarGroup key={g.label ?? i}>
            {g.label ? <SidebarGroupLabel>{g.label}</SidebarGroupLabel> : null}
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
          <SidebarGroupLabel>{PLANNED_GROUP_LABEL}</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {planned.map((it) => (
                <ItemButton
                  key={it.id}
                  item={it}
                  active={active === it.id}
                  planned
                />
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      ) : null}
    </>
  );
}
