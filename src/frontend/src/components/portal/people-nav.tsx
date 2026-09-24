import { useNavigate } from "@tanstack/react-router";
import { Layers, LayoutGrid, Search, User } from "lucide-react";
import { useState } from "react";

import { useViewer } from "@/auth";
import { OrgTree } from "@/components/org-tree";
import { ItemButton } from "@/components/portal/pane-nav";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import { visibleGroups } from "@/lib/insight/groups";
import {
  partitionByReadiness,
  peopleItemsFor,
  resolveZoneItem,
} from "@/lib/portal/nav-model";
import { personSectionPlanned } from "@/lib/portal/nav-policy";
import { usePortalItem, usePortalNavActions } from "@/lib/portal/portal-nav";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { useActiveZone } from "@/lib/portal/use-active-zone";
import { useDismissDrawer } from "@/lib/portal/use-dismiss-drawer";
import {
  usePersonSectionStandings,
  useSelectedPersonSection,
} from "@/lib/portal/use-person-sections";
import { STATUS_BG_CLASS } from "@/lib/status";
import { cn } from "@/lib/utils";
import { useVisibilityPolicy } from "@/queries/identity-me";

export function PeopleNav({
  visibleZones,
  onOpenMe,
}: {
  visibleZones: ReadonlySet<string>;
  onOpenMe: () => void;
}) {
  const { activeZone } = useActiveZone();
  return (
    <>
      <PeopleViews visibleZones={visibleZones} onOpenMe={onOpenMe} />
      {activeZone === "person" ? <PersonSectionsNav /> : null}
      {activeZone === "people" ? <WorkChart /> : null}
    </>
  );
}

function PeopleViews({
  visibleZones,
  onOpenMe,
}: {
  visibleZones: ReadonlySet<string>;
  onOpenMe: () => void;
}) {
  const navigate = useNavigate();
  const { setItem } = usePortalNavActions();
  const dismiss = useDismissDrawer();
  const { activeZone, activePerson } = useActiveZone();
  const { personId } = useViewer();
  const { isFlat } = useVisibilityPolicy();
  const showPlanned = usePortalShowPlanned();
  const item = resolveZoneItem("people", usePortalItem());
  const me = personId ?? activePerson;
  const teamViews = visibleZones.has("people") ? peopleItemsFor(isFlat) : [];
  const { live, planned } = partitionByReadiness(teamViews, showPlanned);

  function openTeamView(id: string) {
    if (activeZone === "people") {
      setItem(id);
      return;
    }
    void navigate({
      to: "/ic/$person/team",
      params: { person: me },
      search: (prev: Record<string, unknown>) => ({
        ...prev,
        item: id,
        acct: undefined,
        zone: undefined,
      }),
    });
  }

  return (
    <SidebarGroup>
      <SidebarGroupLabel>Views</SidebarGroupLabel>
      <SidebarGroupContent>
        <SidebarMenu>
          {visibleZones.has("person") ? (
            <SidebarMenuItem>
              <SidebarMenuButton
                isActive={activeZone === "person" && activePerson === me}
                onClick={() => {
                  onOpenMe();
                  dismiss();
                }}
              >
                <User />
                <span>Me</span>
              </SidebarMenuButton>
            </SidebarMenuItem>
          ) : null}
          {[...live, ...planned].map((view) => (
            <ItemButton
              key={view.id}
              item={view}
              active={activeZone === "people" && item === view.id}
              planned={view.readiness != null}
              onPick={() => openTeamView(view.id)}
            />
          ))}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}

function WorkChart() {
  const [query, setQuery] = useState("");
  // A chart is what a reporting line draws. With no lines there is a roster,
  // and calling it a chart would name a structure the reader cannot see.
  const { isFlat } = useVisibilityPolicy();

  const find = (
    <div className="relative px-2">
      <Search className="pointer-events-none absolute top-1/2 left-4 size-3.5 -translate-y-1/2 text-muted-foreground" />
      <Input
        type="search"
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder="Find someone"
        aria-label="Find someone in the org"
        className="h-8 ps-7 text-sm"
      />
    </div>
  );

  // WORKAROUND: Search stays outside ScrollArea because inertial scrolling can
  // draw its scrollbar and rows over the search.
  return (
    <SidebarGroup className="min-h-0 flex-1">
      {isFlat ? null : <SidebarGroupLabel>WorkChart</SidebarGroupLabel>}
      <SidebarGroupContent className="flex min-h-0 flex-1 flex-col gap-2">
        {find}
        <ScrollArea className="min-h-0 flex-1">
          <OrgTree leadsToTeam query={query} />
        </ScrollArea>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}

function PersonSectionsNav() {
  const { setItem } = usePortalNavActions();
  const dismiss = useDismissDrawer();
  const active = usePortalItem();
  const { activePerson } = useActiveZone();
  // Costs no request: these are the section screens' own queries, so
  // react-query serves them from cache.
  const standings = usePersonSectionStandings(activePerson);
  const standingById = new Map(standings.map((st) => [st.id as string, st]));
  const showPlanned = usePortalShowPlanned();
  const groups = visibleGroups(showPlanned);
  const glance = useSelectedPersonSection() == null;
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
          {groups.map((g) => {
            const standing = standingById.get(g.id as string);
            return (
              <SidebarMenuItem key={g.id}>
                <SidebarMenuButton
                  isActive={active === g.id}
                  className={
                    personSectionPlanned(g.id) ? "text-muted-foreground" : undefined
                  }
                  onClick={() => {
                    setItem(g.id);
                    dismiss();
                  }}
                  title={
                    // Nothing to say until the standings arrive. Both flags
                    // read false while the queries are in flight, so left to
                    // fall through, the tooltip announced the strongest of the
                    // three — that nothing feeds this section — on an answer
                    // the hook had not given. The mark is hidden for that
                    // reason already; the words have to follow it.
                    standing == null || standing.isPending
                      ? undefined
                      : standing.hasData
                        ? standing.phrase
                        : standing.peersHaveData
                          ? "No data this period"
                          : "No data source is connected for this section"
                  }
                >
                  <Layers />
                  <span className="min-w-0 flex-1 truncate">{g.title}</span>
                  {/* The mark that answers "which section is worth opening",
                      beside the thing you click.

                      Three marks, not two, because empty means two different
                      things. A grey dot is a section that reads fine and holds
                      nothing for this person this period — a fact about them.
                      A hollow ring is one nothing feeds — a fact about the
                      install, and not worth opening at all until that changes.
                      Drawn identically, the second sent readers looking for a
                      person's missing work when the connector was the whole
                      story. */}
                  {standing && !standing.isPending ? (
                    <span
                      className={cn(
                        "size-1.5 shrink-0 rounded-full",
                        standing.hasData
                          ? STATUS_BG_CLASS[standing.status]
                          : standing.peersHaveData
                            ? "bg-muted-foreground/30"
                            : "border border-muted-foreground/40"
                      )}
                      aria-hidden
                    />
                  ) : null}
                </SidebarMenuButton>
              </SidebarMenuItem>
            );
          })}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}
