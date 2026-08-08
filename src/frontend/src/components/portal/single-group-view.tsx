import { useMemo } from "react";

import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { CollectionDrilldown } from "@/components/widgets/metric-views/collection-drilldown";
import { MetricActivity } from "@/components/widgets/metric-views/metric-activity";
import { SectionMetricIndex } from "@/components/widgets/metric-views/section-metric-index";
import { usePortalPeriod } from "@/hooks/use-portal-period";
import { groupHasData } from "@/lib/insight/group-data";
import { GROUPS, type GroupId } from "@/lib/insight/groups";
import { finestGrain } from "@/lib/insight/metric-grain";
import { sectionSources } from "@/lib/insight/section-sources";
import { injectCohortPeer } from "@/lib/insight/within-team-peer";
import {
  forEntity,
  type MetricCollectionConfig,
} from "@/lib/metrics/collection";
import { normalizePersonId } from "@/lib/metrics/entity";
import { usePersonCohort } from "@/lib/portal/use-person-cohort";
import { useMetricCollection } from "@/queries/metric-results";

const EMPTY_COLLECTION: MetricCollectionConfig = { metrics: [] };
const CLOSED_ENTITY = { type: "person" as const, ids: [] };

/**
 * One activity class, in as much detail as it can be read.
 *
 * The section is not a second, larger ranking. The overview already said
 * which sections are worth opening, and repeating that judgment here — bigger
 * and in red — showed a reader the finding that brought them here instead of
 * explaining it. What a section owes them is the work itself: what was done,
 * on which days, out of what.
 *
 * So the body is the group's declared composition, then each of its headline
 * metrics rendered at the closest grain that metric declares about itself.
 * Nothing here is configured per group: a metric that can name its artifacts
 * lists them, one that reports a daily counter draws its days, one that
 * carries both sides of a ratio shows the denominator, and one that offers
 * nothing says so.
 */
export function SingleGroupView({
  personId,
  groupId,
}: {
  personId: string;
  groupId: GroupId;
}) {
  const { period, dateRange } = usePortalPeriod();
  const entityId = normalizePersonId(personId);
  const def = GROUPS.find((d) => d.id === groupId) ?? null;

  const data = useMetricCollection(
    def?.collection ?? EMPTY_COLLECTION,
    def ? { type: "person", ids: [entityId] } : CLOSED_ENTITY,
    dateRange,
    // The reader's own previous period. It is the comparison they can act on,
    // and a section that dropped it left them only the cohort's middle —
    // which they neither chose nor can see inside.
    { previousPeriod: period },
  );
  const cohortIds = usePersonCohort(entityId);
  const cohortData = useMetricCollection(
    def && cohortIds.length ? def.collection : EMPTY_COLLECTION,
    cohortIds.length ? { type: "person", ids: cohortIds } : CLOSED_ENTITY,
    dateRange,
  );
  const injectedData = useMemo(
    () => ({
      ...data,
      byKey: injectCohortPeer(data.byKey, cohortData.byKey, cohortIds),
    }),
    [data, cohortData.byKey, cohortIds],
  );

  // Metrics the composition above already draws over time. A daily strip
  // beside a weekly chart of the same counter is the same fact twice, and the
  // chart is the better of the two — it carries the split by tool or by
  // repository that a bare strip cannot.
  //
  // Event-grade metrics are exempt: a list of the commits themselves is not a
  // finer drawing of the chart, it is a different kind of answer.
  const charted = new Set(
    (def?.drilldown ?? []).flatMap((block) =>
      block.view === "timeseries" ? block.metrics : [],
    ),
  );
  // Everything the composition already puts on screen in any form — a chart,
  // a headline card, a distribution. The index below lists what is left, so
  // it must not restate a number the reader can already see; a bare repeat
  // adds nothing, unlike a chart and a strip that answer different questions
  // about the same metric.
  const composed = new Set(
    (def?.drilldown ?? []).flatMap((block) => block.metrics),
  );

  // The metrics that name the class, in the order the def states them. The
  // rest of a collection is supporting arithmetic over the same events —
  // rendering every one of them would say the same day twice under different
  // labels.
  const headline = (def?.card.preview ?? []).flatMap((key) => {
    const metric = injectedData.byKey.get(key);
    if (!metric) return [];
    const grain = finestGrain(metric);
    if (grain == null) return [metric];
    if (grain !== "event" && charted.has(key)) return [];
    return [metric];
  });

  if (!def) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon variant="card" state="empty" label="Unknown group" />
      </div>
    );
  }
  if (data.isPending) return <CenteredSpinner className="min-h-[60vh]" />;
  // A failed fetch must surface as a retryable error, not a drilldown
  // rendered over an empty dataset (same policy as MetricGroupsView).
  if (data.isError) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon variant="card" state="error" onRetry={() => data.refetch()} />
      </div>
    );
  }

  // Nothing at all for this person in this class. The composition blocks
  // would each render their own polite placeholder — a chart saying "no data
  // in this period", a summary card showing a dash, a distribution saying "no
  // values" — and a reader would meet one fact restated in four wordings down
  // a full page. One sentence says it once.
  if (!groupHasData(def, injectedData.byKey, entityId)) {
    return (
      <div className="flex flex-col gap-3 p-4 md:p-6">
        <h1 className="text-xl font-semibold tracking-tight">{def.title}</h1>
        <p className="text-sm text-muted-foreground">
          Nothing recorded here for the selected period.
        </p>
      </div>
    );
  }

  const sources = sectionSources(def, injectedData.byKey, entityId);
  const readable = headline.filter((metric) => finestGrain(metric) != null);
  // Nothing to look into is one fact about the class, not one per metric:
  // three metrics each saying "nothing recorded" reads as three faults.
  const observed = headline.filter(
    (metric) => forEntity(metric, entityId).value != null,
  );

  return (
    <div className="flex flex-col gap-4 p-4 md:p-6">
      <h1 className="text-xl font-semibold tracking-tight">{def.title}</h1>
      {sources.length > 0 ? (
        // What was being watched, next to what it concluded. A low number
        // means one thing when the tool everyone uses is connected and
        // another when it is not, and the reader cannot tell which from the
        // number alone.
        <p className="-mt-2 text-xs text-muted-foreground">
          From {sources.join(", ")}
        </p>
      ) : null}
      <CollectionDrilldown
        def={def}
        data={injectedData}
        entityId={entityId}
        range={dateRange}
      >
        {headline.length > 0 ? (
          <div className="rounded-xl border p-4 sm:p-5">
            <h2 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              The work behind these numbers
            </h2>
            {observed.length === 0 ? (
              <p className="pt-2 text-xs text-muted-foreground">
                Nothing recorded in this section for the selected period.
              </p>
            ) : readable.length === 0 ? (
              // Said once at the top rather than repeated under every metric:
              // when a whole class reports period totals only, that is a fact
              // about the class, and stating it per metric reads as several
              // separate faults.
              <p className="pt-2 text-xs text-muted-foreground">
                These metrics report period totals only — nothing here can be
                broken down by day or by item yet.
              </p>
            ) : (
              observed.map((metric) => (
                <MetricActivity
                  key={metric.metric_key}
                  metric={metric}
                  previous={
                    injectedData.previousByKey?.get(metric.metric_key) ?? null
                  }
                  entityId={entityId}
                  periodNoun={period}
                />
              ))
            )}
          </div>
        ) : null}
        <SectionMetricIndex
          collection={def.collection}
          byKey={injectedData.byKey}
          entityId={entityId}
          shown={new Set([...composed, ...observed.map((m) => m.metric_key)])}
        />
      </CollectionDrilldown>
    </div>
  );
}
