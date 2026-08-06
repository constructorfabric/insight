import { useNavigate } from "@tanstack/react-router";
import { Check, ChevronsUpDown, ChevronUp, Users } from "lucide-react";
import { useMemo } from "react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useIcPerson } from "@/queries/ic-dashboard";
import {
  usePortalNavActions,
} from "@/lib/portal/portal-nav";
import { useOrgScope } from "@/lib/portal/use-org-scope";
import { cn } from "@/lib/utils";

/**
 * Person-zone header — the "climb up" + lateral half of the Person ↔ People
 * model. The name is a peer switcher: it lists the person's teammates (their
 * manager's direct reports) so you can hop between people without bouncing back
 * to People. The supervisor chip drills sideways into the manager; "Team" jumps
 * to People — scoped to the person's own reports if they're a manager, else to
 * their manager's team — and that jump also sets the global org scope, so it only
 * renders when the target is a node the scope can actually reach. Everything is
 * route-driven (clears the pinned zone) and sourced from the identity profile;
 * absent fields render nothing.
 */
export function PersonHeader({ person }: { person: string }) {
  const { setScope, setZone } = usePortalNavActions();
  const navigate = useNavigate();
  const { data } = useIcPerson(person);
  // Ids, not emails, since the identity cutover: the same key the route
  // segment, `?scope=` and the metric entity ids carry.
  const supervisorPersonId = data?.parent_person_id ?? null;
  // Fetch the manager to enumerate siblings; the query self-disables on "".
  const { data: manager } = useIcPerson(supervisorPersonId ?? "");
  // Every node the org scope can actually resolve to — identity serves the
  // viewer only their own subtree, so anything outside it is unreachable.
  const { managerNodes } = useOrgScope();
  const scopeRoots = useMemo(
    () => new Set(managerNodes.map((n) => n.person_id.toLowerCase())),
    [managerNodes],
  );

  if (!data) return null;

  const subtitle = [data.job_title, data.department]
    .filter((s) => s && s.trim())
    .join(" · ");
  const supervisorName = data.supervisor_name ?? null;
  const isManager = data.subordinates.length > 0;
  // Manager → their own team; IC → their manager's team (peers).
  const teamTarget = isManager ? data.person_id : supervisorPersonId;
  // An IC viewer's own supervisor sits ABOVE them, outside the subtree identity
  // serves — scoping there resolves to the viewer's (empty) org and the People
  // zone would render "no people". Hide the button instead of dead-ending,
  // matching the IC-aware shell where org zones don't exist at all.
  const canScopeToTeam = teamTarget
    ? scopeRoots.has(teamTarget.toLowerCase())
    : false;

  const peers = [...(manager?.subordinates ?? [])]
    .filter((p) => p.person_id)
    .sort((a, b) =>
      (a.display_name || a.email).localeCompare(b.display_name || b.email),
    );
  const hasPeers = peers.length > 1;

  function goPerson(personId: string) {
    setZone(null);
    void navigate({ to: "/ic/$person/personal", params: { person: personId } });
  }
  function goTeam(personId: string) {
    setZone(null);
    // Jumping to a team makes that node the visible org scope (design §6), so
    // the topbar badge and every org zone agree with where you just landed.
    setScope({ root: personId });
    void navigate({ to: "/ic/$person/team", params: { person: personId } });
  }

  const title = (
    <h1 className="truncate text-lg font-semibold tracking-tight">
      {data.display_name || data.email}
    </h1>
  );

  return (
    <header className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b px-4 py-3 md:px-6">
      <div className="flex min-w-0 flex-col">
        {hasPeers ? (
          <DropdownMenu>
            <DropdownMenuTrigger
              render={
                <button
                  type="button"
                  className="flex min-w-0 items-center gap-1 rounded-md text-left hover:opacity-80"
                  title="Switch teammate"
                >
                  {title}
                  <ChevronsUpDown className="size-4 shrink-0 text-muted-foreground" />
                </button>
              }
            />
            <DropdownMenuContent
              align="start"
              className="max-h-80 w-64 overflow-y-auto"
            >
              <DropdownMenuGroup>
                <DropdownMenuLabel className="text-xs text-muted-foreground">
                  {supervisorName ? `${supervisorName}'s team` : "Team"}
                </DropdownMenuLabel>
                {peers.map((p) => {
                  const active =
                    p.person_id.toLowerCase() === data.person_id.toLowerCase();
                  return (
                    <DropdownMenuItem
                      key={p.person_id}
                      onClick={() => goPerson(p.person_id)}
                      className={cn(active && "font-medium")}
                    >
                      <Check
                        className={cn(
                          "size-4",
                          active ? "opacity-100" : "opacity-0",
                        )}
                      />
                      <span className="truncate">
                        {p.display_name || p.email}
                      </span>
                    </DropdownMenuItem>
                  );
                })}
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        ) : (
          title
        )}
        {subtitle ? (
          <p className="truncate text-xs text-muted-foreground">{subtitle}</p>
        ) : null}
      </div>

      <div className="ml-auto flex items-center gap-2">
        {supervisorPersonId && supervisorName ? (
          <Button
            variant="ghost"
            size="sm"
            className="h-8 gap-1 text-muted-foreground"
            onClick={() => goPerson(supervisorPersonId)}
          >
            <ChevronUp className="size-3.5" />
            <span className="max-w-40 truncate">{supervisorName}</span>
          </Button>
        ) : null}
        {teamTarget && canScopeToTeam ? (
          <Button
            variant="outline"
            size="sm"
            className="h-8 gap-1.5"
            onClick={() => goTeam(teamTarget)}
          >
            <Users className="size-3.5" />
            {isManager ? "Team" : "Peers"}
          </Button>
        ) : null}
      </div>
    </header>
  );
}
