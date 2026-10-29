import { useMemo, type ReactNode } from "react";

import { useViewer } from "@/auth";
import { DomainLensView, Pending } from "@/components/portal/domain-lens-view";
import { HomeDashboards } from "@/components/portal/home-dashboards";
import { MetricGroupsView } from "@/components/portal/metric-groups-view";
import { OwnCoverage } from "@/components/portal/own-coverage";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { visibleGroups, type GroupId } from "@/lib/insight/groups";
import { homeItemShown, useHomeSubject } from "@/lib/portal/home-subject";
import { zoneById } from "@/lib/portal/nav-model";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { useZoneNav } from "@/lib/portal/use-zone-nav";
import { TEXT_TITLE } from "@/lib/type-scale";
import {
  DEFAULT_OVERVIEW_ITEM,
  OVERVIEW_ITEMS,
  OWN_OVERVIEW_ITEMS,
  overviewMetricKeys,
} from "@/lib/portal/overview-configs";

/**
 * Home and the Overview items, each read for the viewer alone or for their
 * team (`useHomeSubject`). The team grid fetch is zone-scoped (union across
 * all items) so switching items never re-spins the loading gate.
 */
export function OverviewView({ item }: { item: string | null }) {
  const gridKeys = useMemo(() => overviewMetricKeys(), []);
  const { subject, isPending } = useHomeSubject();
  const asked = item ?? DEFAULT_OVERVIEW_ITEM;
  // A manager's item would otherwise open on their own view, then switch to
  // the team's once their reach is known.
  if (isPending && asked !== DEFAULT_OVERVIEW_ITEM) {
    return <CenteredSpinner className="min-h-[60vh]" />;
  }
  const shown = homeItemShown(subject, asked) ? asked : DEFAULT_OVERVIEW_ITEM;

  if (shown === DEFAULT_OVERVIEW_ITEM) {
    const rollup = OVERVIEW_ITEMS[DEFAULT_OVERVIEW_ITEM];

    return (
      <div className="@container flex flex-col gap-6 p-4 md:p-6">
        <h1 className={TEXT_TITLE}>Home</h1>
        <OwnGlance />
        <HomeDashboards />
        {subject === "team" && rollup ? (
          <DomainLensView config={rollup} gridKeys={gridKeys} embedded />
        ) : null}
      </div>
    );
  }
  if (subject === "me") return <OwnItem item={shown} />;

  const entry = OVERVIEW_ITEMS[shown];
  if (!entry) {
    return <Pending label={`“${item}” isn't an Overview view yet.`} />;
  }
  return <DomainLensView config={entry} gridKeys={gridKeys} />;
}

function OwnItem({ item }: { item: string }) {
  const { personId } = useViewer();
  const openSection = useOpenOwnSection();
  const groupIds = useVisibleGroupIds();
  const own = OWN_OVERVIEW_ITEMS[item];

  if (!own || own.kind === "home") {
    return <Pending label={`“${item}” isn't an Overview view yet.`} />;
  }
  if (!personId) {
    return <Pending label="There is no person record to show here." />;
  }

  switch (own.kind) {
    case "lens":
      return <DomainLensView config={own.config} personId={personId} />;
    case "attention":
      return (
        <OwnThemePage title={own.title} tagline={own.tagline}>
          <MetricGroupsView
            personId={personId}
            groupIds={groupIds}
            content="attention"
            embedded
            onSelectGroup={openSection}
          />
        </OwnThemePage>
      );
    case "coverage":
      return (
        <OwnThemePage title={own.title} tagline={own.tagline}>
          <OwnCoverage personId={personId} onOpenSection={openSection} />
        </OwnThemePage>
      );
  }
}

function OwnThemePage({
  title,
  tagline,
  children,
}: {
  title: string;
  tagline: string;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-6 p-4 md:p-6">
      <div>
        <h1 className="text-lg font-semibold tracking-tight">{title}</h1>
        <p className="text-sm text-muted-foreground">{tagline}</p>
      </div>
      {children}
    </div>
  );
}

function OwnGlance() {
  const { personId } = useViewer();
  const openSection = useOpenOwnSection();
  const groupIds = useVisibleGroupIds();

  if (!personId) return null;

  return (
    <MetricGroupsView
      personId={personId}
      groupIds={groupIds}
      content="glance"
      embedded
      onSelectGroup={openSection}
    />
  );
}

function useVisibleGroupIds(): GroupId[] {
  const showPlanned = usePortalShowPlanned();

  return visibleGroups(showPlanned).map((group) => group.id);
}

function useOpenOwnSection(): (id: GroupId) => void {
  const { selectZone } = useZoneNav();
  const personZone = zoneById("person");

  return (id) => {
    if (personZone) selectZone(personZone, id);
  };
}
