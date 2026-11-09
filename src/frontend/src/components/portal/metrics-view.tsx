import { useMemo, useState, type SetStateAction } from "react";
import { Columns3, PanelLeftClose, PanelLeftOpen, X } from "lucide-react";

import { useViewer } from "@/auth";
import { MetricsComparePicker } from "@/components/portal/metrics-compare-picker";
import { MetricsCompareTable } from "@/components/portal/metrics-compare-table";
import {
  MetricsTable,
  type MetricsSort,
} from "@/components/portal/metrics-table";
import { orgScopeGate } from "@/components/portal/org-scope-gate";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import {
  ReportMetricPicker,
  type OfferedReportMetric,
} from "@/components/portal/report-builder-metrics";
import type { MembersGridMember } from "@/components/widgets/dashboard/members-grid";
import {
  Button,
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@gears-frontx/ui-kit";
import { usePortalPeriod } from "@/hooks/use-portal-period";
import { personDisplayName } from "@/lib/identities/person-display";
import { flattenSubordinates } from "@/lib/insight/identity-tree";
import { metricHelp, type MetricHelpText } from "@/lib/insight/metric-help";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import { normalizePersonId } from "@/lib/metrics/entity";
import {
  compareTeams,
  keysWithData,
  metricGroups,
  metricsRows,
  metricsViewCatalog,
  sortRows,
} from "@/lib/portal/metrics-view";
import {
  setMetricsViewPaneShown,
  useMetricsViewPaneShown,
} from "@/lib/portal/metrics-view-pane";
import { metricVisible } from "@/lib/portal/nav-policy";
import { usePortalNavActions, usePortalScope } from "@/lib/portal/portal-nav";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { useOrgScope } from "@/lib/portal/use-org-scope";
import { useShellLayout } from "@/lib/portal/use-shell-layout";
import { byFamily } from "@/lib/reports/families";
import { useMetricDefinitionsResponse } from "@/queries/metric-definitions";
import { useMetricsViewData } from "@/queries/metrics-view";
import type { IdentityPerson } from "@/types/insight";

const TEST_ID = "page-people-metrics";

export function MetricsView() {
  const { dateRange } = usePortalPeriod();
  const orgScope = useOrgScope();
  const definitions = useMetricDefinitionsResponse();
  const showPlanned = usePortalShowPlanned();

  const catalog = useMemo(
    () =>
      metricsViewCatalog(definitions.data?.metrics ?? [], (key) =>
        metricVisible(key, showPlanned),
      ),
    [definitions.data, showPlanned],
  );
  const catalogKeys = useMemo(
    () => catalog.map((metric) => metric.metric_key),
    [catalog],
  );
  const helpByKey = useMemo(
    () =>
      new Map(
        catalog.flatMap((metric) => {
          const help = metricHelp(metric);
          return help ? [[metric.metric_key, help] as const] : [];
        }),
      ),
    [catalog],
  );
  const families = useMemo(
    () =>
      byFamily<OfferedReportMetric>(
        catalog.map((metric) => ({ ...metric, reason: null })),
      ),
    [catalog],
  );

  const [picked, setPicked] = useState<string[] | null>(null);
  const selected = picked ?? catalogKeys;
  const setSelected = (action: SetStateAction<string[]>) =>
    setPicked((current) =>
      typeof action === "function" ? action(current ?? catalogKeys) : action,
    );
  const metricKeys = useMemo(() => {
    const chosen = new Set(selected);
    return catalogKeys.filter((key) => chosen.has(key));
  }, [catalogKeys, selected]);

  const { pivot, roster } = orgScope;
  const people = useMemo(
    () => (pivot ? [pivot, ...flattenSubordinates(pivot)] : (roster ?? [])),
    [pivot, roster],
  );
  const members = useMemo<MembersGridMember[]>(
    () =>
      people.map((person) => ({
        entityId: normalizePersonId(person.person_id),
        displayName: personDisplayName(person),
      })),
    [people],
  );
  const memberIds = useMemo(
    () => members.map((member) => member.entityId),
    [members],
  );

  const data = useMetricsViewData(metricKeys, memberIds, dateRange);

  const gate = orgScopeGate({
    "data-testid": TEST_ID,
    viewerLoading: orgScope.isLoading,
    viewerError: orgScope.isError,
    membersLoading: false,
    membersError: false,
    memberCount: members.length,
    gridPending: definitions.isPending,
    gridError: definitions.isError || data.isError,
    emptyLabel: "No people in the current scope.",
    onRetry: () => {
      orgScope.refetch();
      data.refetch();
    },
  });

  return (
    <div data-testid={TEST_ID} className="flex flex-col gap-3 p-4 md:p-6">
      <div className="flex flex-wrap items-center gap-2">
        <PaneToggle />
        <h1 className="text-lg font-semibold tracking-tight">Metrics view</h1>
        <ScopeChip label={orgScope.label} />
        <Popover>
          <PopoverTrigger
            render={
              <Button
                type="button"
                variant="outline"
                size="sm"
                icon={<Columns3 />}
                className="ms-auto"
              >
                {`Metrics: ${metricKeys.length} of ${catalogKeys.length}`}
              </Button>
            }
          />
          <PopoverContent
            align="end"
            className="max-h-[70svh] w-[min(56rem,90vw)] overflow-y-auto p-0"
          >
            <div className="flex items-center justify-end gap-2 border-b px-4 py-2">
              <Button
                type="button"
                variant="ghost"
                size="sm"
                aria-label="Select all metrics"
                onClick={() => setPicked(null)}
              >
                Select all
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                aria-label="Clear all metrics"
                onClick={() => setPicked([])}
              >
                Clear all
              </Button>
            </div>
            <ReportMetricPicker
              families={families}
              selected={selected}
              setSelected={setSelected}
            />
          </PopoverContent>
        </Popover>
      </div>
      {gate ?? (
        <MetricsBody
          key={orgScope.pivotPersonId}
          pivot={pivot}
          people={people}
          members={members}
          metricKeys={metricKeys}
          byKey={data.byKey}
          helpByKey={helpByKey}
          loading={data.isPending}
        />
      )}
    </div>
  );
}

function PaneToggle() {
  const shown = useMetricsViewPaneShown();
  if (useShellLayout() !== "wide") return null;

  return (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      icon={shown ? <PanelLeftClose /> : <PanelLeftOpen />}
      onClick={() => setMetricsViewPaneShown(!shown)}
    >
      {shown ? "Hide menu" : "Show menu"}
    </Button>
  );
}

function ScopeChip({ label }: { label: string }) {
  const { root } = usePortalScope();
  const { personId } = useViewer();
  const { setScope } = usePortalNavActions();

  const scopedToSomeoneElse =
    root != null &&
    normalizePersonId(root) !== normalizePersonId(personId ?? "");
  if (!scopedToSomeoneElse) return null;

  return (
    <span className="flex items-center gap-1 rounded-full border bg-muted/50 py-0.5 ps-3 pe-1 text-sm">
      <span className="text-muted-foreground">Scoped:</span>
      <span className="font-medium">{label}</span>
      <button
        type="button"
        aria-label={`Clear scope ${label}`}
        onClick={() => setScope({ root: null })}
        className="flex size-5 cursor-pointer items-center justify-center rounded-full text-muted-foreground hover:bg-muted hover:text-foreground"
      >
        <X className="size-3.5" />
      </button>
    </span>
  );
}

function nextSort(current: MetricsSort | null, key: string): MetricsSort | null {
  if (current?.key !== key) return { key, direction: "descending" };
  if (current.direction === "descending") return { key, direction: "ascending" };
  return null;
}

function MetricsBody({
  pivot,
  people,
  members,
  metricKeys,
  byKey,
  helpByKey,
  loading,
}: {
  pivot: IdentityPerson | null;
  people: ReadonlyArray<{ person_id: string; display_name?: string | null }>;
  members: MembersGridMember[];
  metricKeys: readonly string[];
  byKey: Map<string, NormalizedMetricResult>;
  helpByKey: ReadonlyMap<string, MetricHelpText>;
  loading: boolean;
}) {
  const { setScope } = usePortalNavActions();
  const [comparing, setComparing] = useState<string[] | null>(null);
  const [sort, setSort] = useState<MetricsSort | null>(null);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(
    () => new Set(pivot ? [normalizePersonId(pivot.person_id)] : []),
  );
  const teams = compareTeams(pivot);
  const allManagers = teams.map((team) => team.id);
  const allExpanded = allManagers.every((id) => expanded.has(id));

  const byId = new Map(members.map((member) => [member.entityId, member]));
  const compared = (comparing ?? []).flatMap((id) => {
    const member = byId.get(id);
    return member ? [member] : [];
  });
  const inView = comparing ? compared : members;
  const inViewIds = inView.map((member) => member.entityId);
  const columns = keysWithData(metricKeys, byKey, inViewIds);

  const toggle = (entityId: string) =>
    setExpanded((current) => {
      const next = new Set(current);
      if (!next.delete(entityId)) next.add(entityId);
      return next;
    });

  const rows = sort
    ? sortRows(metricsRows(null, people, expanded), sort.key, sort.direction, byKey)
    : metricsRows(pivot, people, expanded);

  return (
    <section className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-3">
        <p className="text-sm text-muted-foreground">
          {comparing
            ? `Comparing ${inView.length} people · ${columns.length} metrics with data`
            : `${inView.length} people · ${columns.length} metrics with data`}
        </p>
        {allManagers.length > 0 && !sort && !comparing ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => setExpanded(new Set(allExpanded ? [] : allManagers))}
          >
            {allExpanded ? "Collapse all" : "Expand all"}
          </Button>
        ) : null}
        <div className="ms-auto flex items-center gap-2">
          {comparing ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              icon={<X />}
              onClick={() => setComparing(null)}
            >
              Exit compare
            </Button>
          ) : null}
          <MetricsComparePicker
            people={members}
            teams={teams}
            picked={comparing ?? []}
            onCompare={setComparing}
          />
        </div>
      </div>
      {metricKeys.length === 0 ? (
        <EmptyNote text="No metrics picked." />
      ) : loading ? (
        <CenteredSpinner className="min-h-[40vh]" />
      ) : comparing ? (
        <div className="rounded-lg border bg-card">
          <MetricsCompareTable
            people={compared}
            keys={columns}
            byKey={byKey}
            helpByKey={helpByKey}
          />
        </div>
      ) : (
        <MetricsTable
          rows={rows}
          groups={metricGroups(columns)}
          byKey={byKey}
          helpByKey={helpByKey}
          sort={sort}
          onSort={(key) => setSort((current) => nextSort(current, key))}
          onToggle={toggle}
          onScope={(entityId) => setScope({ root: entityId })}
        />
      )}
    </section>
  );
}

function EmptyNote({ text }: { text: string }) {
  return (
    <p className="rounded-lg border bg-card p-6 text-center text-sm text-muted-foreground">
      {text}
    </p>
  );
}
