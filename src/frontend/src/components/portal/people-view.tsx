import { useEffect } from "react";

import { EmployeesView } from "@/components/portal/employees-view";
import { TeamStateView } from "@/components/portal/team-state-view";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { normalizePersonId } from "@/lib/metrics/entity";
import {
  usePortalNavActions,
} from "@/lib/portal/portal-nav";
import { claimScopeSync } from "@/lib/portal/route-scope-sync";

/**
 * People zone content, driven by the selected pane item. The roster is a
 * lead-facing team-state dashboard (state → attention → member scan); the
 * individual view lives in the dedicated Person rail zone (drill into any
 * name), not here. Median-by-Role is an honest scaffold pending the cohort
 * pipeline; Employees is a live identity directory.
 *
 * The People route is the one place where a route still *sets* the org scope:
 * landing on /ic/<person_id>/team (a link, a drill, or a pasted URL) makes that
 * node the visible scope, after which every org zone reads it from the URL.
 */
export function PeopleView({
  person,
  item,
}: {
  person: string;
  item: string | null;
}) {
  const { replaceScope } = usePortalNavActions();
  // Sync route → scope once per person, not on every render or remount: the
  // effect must not fight a scope the user then changes from the topbar.
  useEffect(() => {
    // Normalised before the guard sees it: the same person arrives from a link,
    // an identity record or a hand-edited URL in either case, and an
    // unnormalised compare re-syncs the scope on a casing difference alone.
    const id = person ? normalizePersonId(person) : "";
    if (claimScopeSync(id)) replaceScope({ root: id });
  }, [person, replaceScope]);

  if (item === "median-by-role") {
    return (
      <Pending label="Medians by role — not built yet" />
    );
  }
  if (item === "employees") {
    return <EmployeesView />;
  }
  // Default (roster): the team-state dashboard over the active org scope.
  return <TeamStateView />;
}

function Pending({ label }: { label: string }) {
  return (
    <div className="mx-auto w-full max-w-md p-8">
      <ComingSoon variant="card" state="empty" label={label} />
    </div>
  );
}
