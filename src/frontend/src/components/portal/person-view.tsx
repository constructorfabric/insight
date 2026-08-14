import { MetricGroupsView } from "@/components/portal/metric-groups-view";
import { PersonHeader } from "@/components/portal/person-header";
import { SingleGroupView } from "@/components/portal/single-group-view";
import { GROUPS, type GroupId } from "@/lib/insight/groups";
import { usePortalItem, usePortalNavActions } from "@/lib/portal/portal-nav";

const PERSON_GROUP_IDS: readonly GroupId[] = GROUPS.map((g) => g.id);

/**
 * Person zone: one specific person.
 *
 * The second sidebar level lists the person's sections and carries a mark
 * beside each saying which is worth opening; selecting one expands it into the
 * content area inline, with no modal. "At a glance" is the overview: the
 * headline row and what needs attention, both routing into the section that
 * owns the number.
 *
 * The per-section status cards this page used to carry are gone — they said
 * again, in the middle of the page, what the navigation says on its left edge.
 */
export function PersonView({ person }: { person: string }) {
  const item = usePortalItem();
  const { setItem } = usePortalNavActions();
  const isSection =
    item != null && (PERSON_GROUP_IDS as string[]).includes(item);

  return (
    <>
      <PersonHeader person={person} />
      {isSection ? (
        <SingleGroupView personId={person} groupId={item as GroupId} />
      ) : (
        // "At a glance" — the headline row and what needs attention.
        //
        // A tile or an alert opens the SECTION that owns it, the same one the
        // navigation lists: the number is the way in, and the section is where
        // it is explained. No modal — a problem points you straight at the
        // screen that covers it.
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
