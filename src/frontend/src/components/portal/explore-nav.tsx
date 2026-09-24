import { Layers } from "lucide-react";
import { useState } from "react";

import { CountBadge, ItemBadge, ItemButton } from "@/components/portal/pane-nav";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
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
  partitionByReadiness,
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

export function ExploreNav({ visibleZones }: { visibleZones: ReadonlySet<string> }) {
  return (
    <>
      {visibleZones.has("directions") ? <DirectionsGroup /> : null}
      {visibleZones.has("aicost") ? <AiCostGroup /> : null}
    </>
  );
}

function DirectionsGroup() {
  const showPlanned = usePortalShowPlanned();
  const directions = visibleDirections(showPlanned);
  return (
    <SidebarGroup>
      <SidebarGroupLabel>Directions</SidebarGroupLabel>
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
  const dismiss = useDismissDrawer();
  const { activeZone } = useActiveZone();
  const activeDir = usePortalDir();
  const activeLens = usePortalLens();
  const showPlanned = usePortalShowPlanned();
  const expanded = activeZone === "directions" && activeDir === direction.id;
  const Icon = direction.icon;
  const lenses = visibleLenses(direction, showPlanned);

  function toggle() {
    if (expanded) {
      setDir("");
    } else {
      openDirection(direction.id, lenses[0] ?? direction.lenses[0]!);
    }
  }

  return (
    <>
      <SidebarMenuItem>
        <SidebarMenuButton
          isActive={expanded}
          onClick={toggle}
          aria-expanded={expanded}
        >
          <Icon />
          <span>{direction.name}</span>
          <CountBadge>{lenses.length}</CountBadge>
        </SidebarMenuButton>
      </SidebarMenuItem>

      {expanded ? (
        <SidebarMenuSub>
          {lenses.map((lens) => (
            <SidebarMenuSubItem key={lens}>
              <SidebarMenuSubButton
                isActive={activeLens === lens}
                className={
                  lensRoadmap(direction, lens) ? "text-muted-foreground" : undefined
                }
                onClick={() => {
                  openDirection(direction.id, lens);
                  dismiss();
                }}
              >
                <span>{lens}</span>
              </SidebarMenuSubButton>
            </SidebarMenuSubItem>
          ))}
        </SidebarMenuSub>
      ) : null}
    </>
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
      <SidebarGroupLabel>AI &amp; Cost</SidebarGroupLabel>
      <SidebarGroupContent>
        <SidebarMenu>
          {zoneSections("aicost").map((group, i) => {
            if (group.label) {
              return <AiCostFolder key={group.label} group={group} active={active} />;
            }
            const { live, planned } = partitionByReadiness(group.items, showPlanned);
            return [...live, ...planned].map((item) => (
              <ItemButton
                key={`${i}-${item.id}`}
                item={item}
                active={active === item.id}
                planned={item.readiness != null}
                onPick={() => openItem("aicost", item.id)}
              />
            ));
          })}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}

function AiCostFolder({ group, active }: { group: PaneGroup; active: string | null }) {
  const { openItem } = usePortalNavActions();
  const dismiss = useDismissDrawer();
  const showPlanned = usePortalShowPlanned();
  const [collapsed, setCollapsed] = useState(false);
  const { live, planned } = partitionByReadiness(group.items, showPlanned);
  const items = [...live, ...planned];
  const holdsActive = items.some((item) => item.id === active);
  const expanded = holdsActive && !collapsed;
  const Icon = group.icon ?? Layers;

  if (!items.length) return null;

  function toggle() {
    if (expanded) {
      setCollapsed(true);
      return;
    }
    setCollapsed(false);
    if (!holdsActive) openItem("aicost", items[0]!.id);
  }

  return (
    <>
      <SidebarMenuItem>
        <SidebarMenuButton isActive={expanded} onClick={toggle} aria-expanded={expanded}>
          <Icon />
          <span>{group.label}</span>
          <CountBadge>{items.length}</CountBadge>
        </SidebarMenuButton>
      </SidebarMenuItem>

      {expanded ? (
        <SidebarMenuSub>
          {items.map((item) => (
            <SidebarMenuSubItem key={item.id}>
              <SidebarMenuSubButton
                isActive={active === item.id}
                className={item.readiness != null ? "text-muted-foreground" : undefined}
                onClick={() => {
                  openItem("aicost", item.id);
                  dismiss();
                }}
              >
                <span>{item.label}</span>
                <ItemBadge item={item} />
              </SidebarMenuSubButton>
            </SidebarMenuSubItem>
          ))}
        </SidebarMenuSub>
      ) : null}
    </>
  );
}
