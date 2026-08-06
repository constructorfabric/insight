import { MetricGroupsView } from "@/components/portal/metric-groups-view";
import { PersonHeader } from "@/components/portal/person-header";
import { SingleGroupView } from "@/components/portal/single-group-view";
import { GROUPS, type GroupId } from "@/lib/insight/groups";
import {
  usePortalItem,
  usePortalNavActions,
} from "@/lib/portal/portal-nav";

const PERSON_GROUP_IDS: readonly GroupId[] = GROUPS.map((g) => g.id);

/**
 * Person zone: one specific person (like the reporting tool's Person page).
 * The second sidebar level lists the person's sections; selecting a section
 * expands it into the full content area inline (rich drilldown) — no modal.
 * "At a glance" is a health dashboard: KPI row + needs-attention + per-section
 * status cards, all routing into the relevant section inline so a problem
 * points straight at its section.
 */
export function PersonView({ person }: { person: string }) {
  const { setItem } = usePortalNavActions();
  const item = usePortalItem();
  const isSection = item != null && (PERSON_GROUP_IDS as string[]).includes(item);

  return (
    <>
      <PersonHeader person={person} />
      {isSection ? (
        <SingleGroupView personId={person} groupId={item as GroupId} />
      ) : (
        // "At a glance" — a health dashboard: general KPIs + needs-attention +
        // per-section status cards. Everything routes into the relevant section
        // inline (no modal), so a problem points you straight at its section.
        <MetricGroupsView
          personId={person}
          groupIds={PERSON_GROUP_IDS}
          showKpis
          onSelectGroup={(id) => setItem(id)}
        />
      )}
    </>
  );
}
