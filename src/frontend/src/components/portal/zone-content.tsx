import { ComingSoon } from "@/components/widgets/coming-soon";
import { AiCostView } from "@/components/portal/ai-cost-view";
import { DirectionView } from "@/components/portal/direction-view";
import { OverviewView } from "@/components/portal/overview-view";
import { ManageView } from "@/components/portal/manage-view";
import { PeopleView } from "@/components/portal/people-view";
import { PersonView } from "@/components/portal/person-view";
import { zoneById } from "@/lib/portal/nav-model";
import {
  usePortalDir,
  usePortalItem,
  usePortalLens,
} from "@/lib/portal/portal-nav";
import { useActiveZone } from "@/lib/portal/use-active-zone";

/**
 * Content for the non-entity portal zones. Overview rolls the org up; Directions
 * route each lens to a focused domain view or an honest ComingSoon; Manage reads
 * the live catalog; Scorecard / Reports are honest scaffolds.
 *
 * Org zones take no person prop — they read the global org scope (design §6) via
 * `useOrgScope`, so the topbar badge and every org subtitle always agree. Only
 * the entity lenses (Person / People) stay route-driven.
 */
export function ZoneContent() {
  const { activeZone, activePerson } = useActiveZone();
  const dir = usePortalDir();
  const lens = usePortalLens();
  const item = usePortalItem();

  switch (activeZone) {
    case "person":
      return <PersonView person={activePerson} />;
    case "overview":
      return <OverviewView item={item} />;
    case "directions":
      return <DirectionView dir={dir} lens={lens} />;
    case "aicost":
      return <AiCostView item={item} />;
    case "people":
      return <PeopleView person={activePerson} item={item} />;
    case "manage":
      return <ManageView item={item} />;
    case "scorecard":
    case "reports":
      return <ZoneScaffold zone={activeZone} />;
    default:
      return <ZoneScaffold zone={activeZone} />;
  }
}

const PENDING_BY_ZONE: Record<string, string> = {
  scorecard: "org snapshots + unit × quarter aggregation",
  reports: "diagnosis circuit + report builder",
};

function ZoneScaffold({ zone }: { zone: string }) {
  // Keyed, not defaulted: an unrecognised zone from the URL used to read
  // "pending: diagnosis circuit + report builder", which names work that has
  // nothing to do with it.
  const pending =
    PENDING_BY_ZONE[zone] ?? "this lens";
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-4 p-8">
      <div className="flex flex-col items-center gap-1 text-center">
        <h1 className="text-lg font-semibold tracking-tight">
          {zoneById(zone)?.label ?? "Portal"}
        </h1>
        <p className="max-w-md text-sm text-muted-foreground">
          Portal lens — structure in place, wiring pending.
        </p>
      </div>
      <div className="w-full max-w-md">
        <ComingSoon variant="card" state="empty" label={`Pending: ${pending}`} />
      </div>
    </div>
  );
}
