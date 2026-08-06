import { ChevronRight, Layers, LayoutGrid, Settings2 } from "lucide-react";
import { useState } from "react";

import { AppSidebarFooter } from "@/components/app-sidebar-footer";
import { OrgTree } from "@/components/org-tree";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { GROUPS } from "@/lib/insight/groups";
import { lensEntry } from "@/lib/portal/lens-configs";
import { useShellLayout } from "@/lib/portal/use-shell-layout";
import { useZoneNav } from "@/lib/portal/use-zone-nav";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarMenuSubButton,
  SidebarMenuSubItem,
  useSidebar,
} from "@/components/ui/sidebar";
import {
  DIRECTIONS,
  MANAGE_ITEMS,
  PEOPLE_ITEMS,
  PLANNED_GROUP_LABEL,
  partitionByReadiness,
  ZONE_SECTIONS,
  zoneById,
  type Direction,
  type PaneItem,
} from "@/lib/portal/nav-model";
import {
  usePortalShowPlanned,
} from "@/lib/portal/portal-store";
import {
  usePortalDir,
  usePortalItem,
  usePortalLens,
  usePortalNavActions,
} from "@/lib/portal/portal-nav";
import { useActiveZone } from "@/lib/portal/use-active-zone";
import { cn } from "@/lib/utils";

const ZONE_SUB: Record<string, string> = {
  overview: "Cross-functional org rollup",
  directions: "Functional domains",
  person: "Pick a person",
  people: "Roster & org structure",
  aicost: "Adoption funnel & cost",
  scorecard: "Unit × quarter × QoQ",
  reports: "Generated & custom",
  manage: "Catalog, identity & governance",
};

const BADGE_TONE: Record<string, string> = {
  warn: "bg-warning/15 text-warning",
  new: "bg-primary/10 text-foreground",
  error: "bg-destructive/15 text-destructive",
};

/**
 * Zone-contextual secondary navigation, driven by the active rail zone.
 *
 * On a phone this is the ONLY navigation surface: the icon rail hides itself
 * (two fixed sidebars left ~60px for content), so the pane becomes an
 * off-canvas drawer — opened by the topbar trigger — and carries the zone list
 * and the settings menu that normally live in the rail. Desktop is unchanged:
 * `collapsible="none"`, in normal flow, zones in the rail beside it.
 */
export function ContextPane() {
  const layout = useShellLayout();
  // A phone hides the rail, so the drawer inherits its duties. A tablet keeps
  // the rail — the drawer there is only the pane itself, collapsed to give the
  // content its 256px back.
  const isPhone = layout === "phone";
  const drawer = layout !== "wide";
  const { activeZone } = useActiveZone();
  const zone = zoneById(activeZone);
  const title = zone?.label ?? "Insight";

  return (
    <Sidebar collapsible={drawer ? "offcanvas" : "none"} className="border-e">
      {/* The drawer's zone row already names the zone, so repeating it in a
          header would cost two of the ~14 rows a phone has. */}
      {isPhone ? null : (
        <SidebarHeader>
          <div className="flex flex-col px-2 py-1.5">
            <span className="text-sm font-semibold tracking-tight text-sidebar-foreground">
              {title}
            </span>
            <span className="text-xs text-muted-foreground">
              {ZONE_SUB[activeZone] ?? ""}
            </span>
          </div>
        </SidebarHeader>
      )}
      <SidebarContent>
        {isPhone ? <MobileZoneNav /> : null}
        {activeZone === "directions" ? (
          <DirectionsNav />
        ) : activeZone === "people" ? (
          <PeopleNav />
        ) : activeZone === "manage" ? (
          <ItemsNav items={MANAGE_ITEMS} groupLabel="Manage" />
        ) : activeZone === "person" ? (
          <PersonSectionsNav />
        ) : (
          <ThemeNav zoneId={activeZone} />
        )}
      </SidebarContent>
      {isPhone ? (
        <SidebarFooter>
          {/* One row, not six: inline the settings menu and it takes a third of
              the drawer, crowding out the sections that are the point of it.
              Same affordance the rail gives desktop — an icon that opens the
              menu on demand. */}
          <SidebarMenu>
            <SidebarMenuItem>
              <Popover>
                <PopoverTrigger
                  render={
                    <SidebarMenuButton>
                      <Settings2 aria-hidden />
                      <span>Settings</span>
                    </SidebarMenuButton>
                  }
                />
                <PopoverContent side="top" align="start" className="w-60 gap-0 p-1">
                  <AppSidebarFooter />
                </PopoverContent>
              </Popover>
            </SidebarMenuItem>
          </SidebarMenu>
        </SidebarFooter>
      ) : null}
    </Sidebar>
  );
}

/**
 * Dismiss the mobile drawer after a LEAF pick (a section / lens / group): on a
 * phone the pane is the drawer, so leaving it open would hide the very view the
 * reader just chose. Zone picks deliberately keep it open — the zone's items
 * render right below, so zone-then-item is one pass. No-op on desktop, where
 * the pane is always-visible chrome.
 */
function useDismissDrawer(): () => void {
  const layout = useShellLayout();
  const { setOpen, setOpenMobile } = useSidebar();
  return () => {
    // Below 768 the pane is a Sheet (`openMobile`); on a tablet it is an
    // off-canvas panel (`open`). Wide keeps it in flow — nothing to dismiss.
    if (layout === "phone") setOpenMobile(false);
    else if (layout === "narrow") setOpen(false);
  };
}

/**
 * Zone switcher for the mobile drawer, standing in for the hidden icon rail.
 *
 * Collapsed to a SINGLE row by default — the full list is eight zones tall, and
 * expanded it pushed the zone's own sections below the fold, so picking a zone
 * looked like it did nothing. Collapsed, the sections start right under this row:
 * changing section (the common move) costs no scrolling, and switching zone
 * costs one extra tap that also re-collapses the list.
 */
function MobileZoneNav() {
  const { zones, activeZone, selectZone } = useZoneNav();
  const [expanded, setExpanded] = useState(false);
  const current = zones.find((z) => z.id === activeZone);
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
              <span className="font-medium">{current?.label ?? "Zones"}</span>
              <ChevronRight
                className={cn(
                  "ms-auto transition-transform",
                  expanded && "rotate-90",
                )}
                aria-hidden
              />
            </SidebarMenuButton>
          </SidebarMenuItem>
          {expanded
            ? zones.map((z) => (
                <SidebarMenuItem key={z.id}>
                  <SidebarMenuButton
                    isActive={activeZone === z.id}
                    onClick={() => {
                      selectZone(z);
                      setExpanded(false);
                    }}
                    className="ps-4"
                  >
                    <z.icon aria-hidden />
                    <span>{z.label}</span>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              ))
            : null}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}

/* ── Theme zones (Overview / AI & Cost / Scorecard / Reports) ────────── */

function ThemeNav({ zoneId }: { zoneId: string }) {
  const groups = ZONE_SECTIONS[zoneId] ?? [];
  const active = usePortalItem();
  const showPlanned = usePortalShowPlanned();
  // Everything not yet real is pulled out of its original group and collected
  // under one demoted "Planned" group at the bottom, so the working menu reads
  // clean and roadmap items stay honest instead of masquerading as features.
  const split = groups.map((g) => partitionByReadiness(g.items, showPlanned));
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
        ) : null,
      )}
      {planned.length ? (
        <SidebarGroup>
          <SidebarGroupLabel>{PLANNED_GROUP_LABEL}</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {planned.map((it) => (
                <ItemButton key={it.id} item={it} active={active === it.id} planned />
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      ) : null}
    </>
  );
}

function ItemsNav({
  items,
  groupLabel,
}: {
  items: readonly PaneItem[];
  groupLabel: string;
}) {
  const active = usePortalItem();
  const showPlanned = usePortalShowPlanned();
  const { live, planned } = partitionByReadiness(items, showPlanned);
  return (
    <>
      {/* Skipped when empty, as ThemeNav does: with planned items hidden a
          zone can have no live items, and a bare heading reads as a load
          failure rather than as a filter. */}
      {live.length ? (
        <SidebarGroup>
          <SidebarGroupLabel>{groupLabel}</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {live.map((it) => (
                <ItemButton key={it.id} item={it} active={active === it.id} />
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      ) : null}
      {planned.length ? (
        <SidebarGroup>
          <SidebarGroupLabel>{PLANNED_GROUP_LABEL}</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {planned.map((it) => (
                <ItemButton key={it.id} item={it} active={active === it.id} planned />
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      ) : null}
    </>
  );
}

function ItemButton({
  item,
  active,
  planned = false,
}: {
  item: PaneItem;
  active: boolean;
  /** Demoted rendering: same affordance, visibly lighter weight. */
  planned?: boolean;
}) {
  const { setItem } = usePortalNavActions();
  const Icon = item.icon;
  const dismiss = useDismissDrawer();
  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        isActive={active}
        onClick={() => {
          setItem(item.id);
          dismiss();
        }}
        className={planned ? "text-muted-foreground" : undefined}
      >
        <Icon />
        <span>{item.label}</span>
        {item.badge ? (
          <span
            className={cn(
              "ml-auto rounded-full px-1.5 py-0.5 text-[9px] font-semibold",
              BADGE_TONE[item.badge.tone],
            )}
          >
            {item.badge.text}
          </span>
        ) : null}
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}

/* ── Directions zone ─────────────────────────────────────────────────── */

function DirectionsNav() {
  return (
    <SidebarGroup>
      <SidebarGroupLabel>
        Directions
        <span className="ml-1 text-[10px] font-normal text-muted-foreground">
          · catalog · {DIRECTIONS.length}
        </span>
      </SidebarGroupLabel>
      <SidebarGroupContent>
        <SidebarMenu>
          {DIRECTIONS.map((d) => (
            <DirectionItem key={d.id} direction={d} />
          ))}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}

function DirectionItem({ direction }: { direction: Direction }) {
  const { setDir, setItem, setLens } = usePortalNavActions();
  const dismiss = useDismissDrawer();
  const activeDir = usePortalDir();
  const activeLens = usePortalLens();
  const showPlanned = usePortalShowPlanned();
  const expanded = activeDir === direction.id;
  const Icon = direction.icon;
  // A lens we simply have not built yet is hidden unless the viewer asked for
  // planned work; a lens waiting on the product stays listed (dimmed) because
  // it tells the reader the domain exists in our model.
  const lenses = direction.lenses.filter((lens) => {
    const entry = lensEntry(direction.id, lens);
    if (!entry || !("comingSoon" in entry)) return true;
    return entry.readiness === "planned" || showPlanned;
  });

  function toggle() {
    if (expanded) {
      setDir("");
    } else {
      setDir(direction.id);
      setLens(lenses[0] ?? direction.lenses[0]!);
    }
  }

  return (
    <>
      <SidebarMenuItem>
        <SidebarMenuButton isActive={expanded} onClick={toggle} aria-expanded={expanded}>
          <Icon />
          <span>{direction.name}</span>
          {direction.source === "bullet" ? (
            <span className="ml-auto rounded-full bg-warning/15 px-1.5 py-0.5 text-[9px] font-semibold text-warning">
              bullet
            </span>
          ) : null}
          <ChevronRight
            className={cn(
              "size-4 text-muted-foreground transition-transform",
              direction.source === "bullet" ? "ml-1" : "ml-auto",
              expanded && "rotate-90",
            )}
          />
        </SidebarMenuButton>
      </SidebarMenuItem>

      {expanded ? (
        <>
          <SidebarMenuSub>
            {lenses.map((lens) => {
              const entry = lensEntry(direction.id, lens);
              const roadmap = !!entry && "comingSoon" in entry;
              return (
                <SidebarMenuSubItem key={lens}>
                  <SidebarMenuSubButton
                    isActive={activeLens === lens}
                    className={roadmap ? "text-muted-foreground" : undefined}
                    onClick={() => {
                      setLens(lens);
                      setItem(null);
                      dismiss();
                    }}
                  >
                    <span>{lens}</span>
                  </SidebarMenuSubButton>
                </SidebarMenuSubItem>
              );
            })}
          </SidebarMenuSub>
        </>
      ) : null}
    </>
  );
}

/* ── People / Person zones ───────────────────────────────────────────── */

function PeopleNav() {
  const active = usePortalItem();
  return (
    <>
      <SidebarGroup>
        <SidebarGroupLabel>Views</SidebarGroupLabel>
        <SidebarGroupContent>
          <SidebarMenu>
            {PEOPLE_ITEMS.map((it) => (
              <ItemButton key={it.id} item={it} active={active === it.id} />
            ))}
          </SidebarMenu>
        </SidebarGroupContent>
      </SidebarGroup>
      <WorkChart />
    </>
  );
}

function WorkChart() {
  return (
    <SidebarGroup>
      <SidebarGroupLabel>WorkChart</SidebarGroupLabel>
      <SidebarGroupContent>
        <OrgTree leadsToTeam />
      </SidebarGroupContent>
    </SidebarGroup>
  );
}

/* ── Person zone: one person, section switcher (no WorkChart, no modal) ─── */

function PersonSectionsNav() {
  const { setItem } = usePortalNavActions();
  const dismiss = useDismissDrawer();
  const active = usePortalItem();
  const groups = GROUPS;
  const groupIds = groups.map((g) => g.id) as string[];
  const glance = active == null || !groupIds.includes(active);
  return (
    <SidebarGroup>
      <SidebarGroupLabel>Sections</SidebarGroupLabel>
      <SidebarGroupContent>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton
              isActive={glance}
              onClick={() => {
                setItem(null);
                dismiss();
              }}
            >
              <LayoutGrid />
              <span>At a glance</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
          {groups.map((g) => (
            <SidebarMenuItem key={g.id}>
              <SidebarMenuButton
                isActive={active === g.id}
                onClick={() => {
                  setItem(g.id);
                  dismiss();
                }}
              >
                <Layers />
                <span>{g.title}</span>
              </SidebarMenuButton>
            </SidebarMenuItem>
          ))}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}
