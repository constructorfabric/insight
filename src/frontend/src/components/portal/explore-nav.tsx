import { Layers, type LucideIcon } from "lucide-react";
import { useState, type ReactNode } from "react";

import { CountBadge, ItemBadge, ItemButton } from "@/components/portal/pane-nav";
import { PaneGroupLabel } from "@/components/portal/pane-group-label";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarMenuSubButton,
  SidebarMenuSubItem,
} from "@/components/ui/sidebar";
import {
  lensRoadmap,
  visibleDirections,
  visibleLenses,
} from "@/lib/portal/lens-configs";
import {
  orderByReadiness,
  resolveZoneItem,
  zoneSections,
  type Direction,
  type PaneGroup,
} from "@/lib/portal/nav-model";
import {
  usePortalDir,
  usePortalItem,
  usePortalLens,
  usePortalNavActions,
} from "@/lib/portal/portal-nav";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { useActiveZone } from "@/lib/portal/use-active-zone";
import { useDismissDrawer } from "@/lib/portal/use-dismiss-drawer";
import { cn } from "@/lib/utils";

interface FolderRow {
  key: string;
  label: string;
  active: boolean;
  muted: boolean;
  badge?: ReactNode;
  onPick: () => void;
}

export function ExploreNav({ visibleZones }: { visibleZones: ReadonlySet<string> }) {
  return (
    <>
      {visibleZones.has("directions") ? <DirectionsGroup /> : null}
      {visibleZones.has("aicost") ? <AiCostGroup /> : null}
    </>
  );
}

function Folder({
  "data-planned": planned,
  icon: Icon,
  label,
  expanded,
  onToggle,
  rows,
}: {
  "data-planned"?: boolean;
  icon: LucideIcon;
  label: string;
  expanded: boolean;
  onToggle: () => void;
  rows: readonly FolderRow[];
}) {
  const dismiss = useDismissDrawer();
  return (
    <>
      <SidebarMenuItem>
        <SidebarMenuButton
          data-planned={planned || undefined}
          isActive={expanded}
          onClick={onToggle}
          aria-expanded={expanded}
        >
          <Icon />
          <span>{label}</span>
        </SidebarMenuButton>
        <CountBadge>{rows.length}</CountBadge>
      </SidebarMenuItem>

      {expanded ? (
        <SidebarMenuSub>
          {rows.map((row) => (
            <SidebarMenuSubItem key={row.key}>
              <SidebarMenuSubButton
                render={<button type="button" />}
                data-planned={row.muted || undefined}
                isActive={row.active}
                className={cn(
                  "w-full text-start",
                  row.muted && "text-muted-foreground"
                )}
                onClick={() => {
                  row.onPick();
                  dismiss();
                }}
              >
                <span>{row.label}</span>
                {row.badge}
              </SidebarMenuSubButton>
            </SidebarMenuSubItem>
          ))}
        </SidebarMenuSub>
      ) : null}
    </>
  );
}

function DirectionsGroup() {
  const showPlanned = usePortalShowPlanned();
  const directions = visibleDirections(showPlanned);
  return (
    <SidebarGroup>
      <PaneGroupLabel>Directions</PaneGroupLabel>
      <SidebarGroupContent>
        <SidebarMenu>
          {directions.map((d) => (
            <DirectionItem key={d.id} direction={d} />
          ))}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}

function DirectionItem({ direction }: { direction: Direction }) {
  const { setDir, openDirection } = usePortalNavActions();
  const { activeZone } = useActiveZone();
  const activeDir = usePortalDir();
  const activeLens = usePortalLens();
  const showPlanned = usePortalShowPlanned();
  const expanded = activeZone === "directions" && activeDir === direction.id;
  const lenses = visibleLenses(direction, showPlanned);

  return (
    <Folder
      data-planned={visibleLenses(direction, false).length === 0 || undefined}
      icon={direction.icon}
      label={direction.name}
      expanded={expanded}
      onToggle={() =>
        expanded
          ? setDir("")
          : openDirection(direction.id, lenses[0] ?? direction.lenses[0]!)
      }
      rows={lenses.map((lens) => ({
        key: lens,
        label: lens,
        active: activeLens === lens,
        muted: Boolean(lensRoadmap(direction, lens)),
        onPick: () => openDirection(direction.id, lens),
      }))}
    />
  );
}

function AiCostGroup() {
  const { activeZone } = useActiveZone();
  const { openItem } = usePortalNavActions();
  const showPlanned = usePortalShowPlanned();
  const resolved = resolveZoneItem("aicost", usePortalItem());
  const active = activeZone === "aicost" ? resolved : null;

  return (
    <SidebarGroup>
      <PaneGroupLabel>AI &amp; Cost</PaneGroupLabel>
      <SidebarGroupContent>
        <SidebarMenu>
          {zoneSections("aicost").map((group, i) =>
            group.label ? (
              <AiCostFolder key={group.label} group={group} active={active} />
            ) : (
              orderByReadiness(group.items, showPlanned).map((item) => (
                <ItemButton
                  key={`${i}-${item.id}`}
                  item={item}
                  active={active === item.id}
                  onPick={() => openItem("aicost", item.id)}
                />
              ))
            )
          )}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}

function AiCostFolder({ group, active }: { group: PaneGroup; active: string | null }) {
  const { openItem } = usePortalNavActions();
  const showPlanned = usePortalShowPlanned();
  const [collapsed, setCollapsed] = useState(false);
  const items = orderByReadiness(group.items, showPlanned);
  const holdsActive = items.some((item) => item.id === active);
  const expanded = holdsActive && !collapsed;

  if (!items.length) return null;

  return (
    <Folder
      data-planned={orderByReadiness(group.items, false).length === 0 || undefined}
      icon={group.icon ?? Layers}
      label={group.label ?? ""}
      expanded={expanded}
      onToggle={() => {
        if (expanded) {
          setCollapsed(true);
          return;
        }
        setCollapsed(false);
        if (!holdsActive) openItem("aicost", items[0]!.id);
      }}
      rows={items.map((item) => ({
        key: item.id,
        label: item.label,
        active: active === item.id,
        muted: item.readiness != null,
        badge: <ItemBadge item={item} />,
        onPick: () => openItem("aicost", item.id),
      }))}
    />
  );
}
