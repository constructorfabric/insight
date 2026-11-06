import { useMemo, useState, type SetStateAction } from "react";
import { Columns3, PanelLeftClose, PanelLeftOpen } from "lucide-react";

import { MemberFilter } from "@/components/portal/member-filter";
import {
  selectedMembers,
  type MemberSelection,
} from "@/components/portal/member-selection";
import { MetricsCompareTable } from "@/components/portal/metrics-compare-table";
import { orgScopeGate } from "@/components/portal/org-scope-gate";
import {
  ReportMetricPicker,
  type OfferedReportMetric,
} from "@/components/portal/report-builder-metrics";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Card, CardContent } from "@/components/ui/card";
import {
  MembersGrid,
  type MembersGridMember,
} from "@/components/widgets/dashboard/members-grid";
import {
  Button,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Switch,
} from "@gears-frontx/ui-kit";
import { usePortalPeriod } from "@/hooks/use-portal-period";
import { personDisplayName } from "@/lib/identities/person-display";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import { normalizePersonId } from "@/lib/metrics/entity";
import {
  compareRows,
  keysWithData,
  metricsViewCatalog,
} from "@/lib/portal/metrics-view";
import {
  setMetricsViewPaneShown,
  useMetricsViewPaneShown,
} from "@/lib/portal/metrics-view-pane";
import { metricVisible } from "@/lib/portal/nav-policy";
import { usePortalNavActions } from "@/lib/portal/portal-nav";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { useOrgScope, type ManagerNode } from "@/lib/portal/use-org-scope";
import { byFamily } from "@/lib/reports/families";
import { useMetricDefinitionsResponse } from "@/queries/metric-definitions";
import { useMetricsViewData } from "@/queries/metrics-view";

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
  const members = useMemo<MembersGridMember[]>(
    () =>
      [...(pivot ? [pivot] : []), ...(roster ?? [])].map((person) => ({
        entityId: normalizePersonId(person.person_id),
        displayName: personDisplayName(person),
      })),
    [pivot, roster],
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
    gridPending: definitions.isPending || data.isPending,
    gridError: definitions.isError || data.isError,
    emptyLabel: "No people in the current scope.",
    onRetry: () => {
      orgScope.refetch();
      data.refetch();
    },
  });

  return (
    <div data-testid={TEST_ID} className="flex flex-col gap-4 p-4 md:p-6">
      <div className="flex flex-wrap items-center gap-2">
        <PaneToggle />
        <h1 className="me-auto text-lg font-semibold tracking-tight">Metrics view</h1>
        <TeamSelect managers={orgScope.managerNodes} value={orgScope.pivotPersonId} />
        <Popover>
          <PopoverTrigger
            render={
              <Button type="button" variant="outline" size="sm" icon={<Columns3 />}>
                {`Metrics: ${metricKeys.length} of ${catalogKeys.length}`}
              </Button>
            }
          />
          <PopoverContent
            align="end"
            className="max-h-[70svh] w-[min(56rem,90vw)] overflow-y-auto p-0"
          >
            <ReportMetricPicker
              families={families}
              selected={selected}
              setSelected={setSelected}
            />
          </PopoverContent>
        </Popover>
      </div>
      {gate ?? (
        <MetricsTable
          key={orgScope.pivotPersonId}
          members={members}
          metricKeys={metricKeys}
          byKey={data.byKey}
        />
      )}
    </div>
  );
}

function PaneToggle() {
  const shown = useMetricsViewPaneShown();

  return (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      className="hidden lg:inline-flex"
      icon={shown ? <PanelLeftClose /> : <PanelLeftOpen />}
      onClick={() => setMetricsViewPaneShown(!shown)}
    >
      {shown ? "Hide menu" : "Show menu"}
    </Button>
  );
}

function TeamSelect({
  managers,
  value,
}: {
  managers: readonly ManagerNode[];
  value: string;
}) {
  const { setScope } = usePortalNavActions();
  if (managers.length === 0) return null;

  const current = managers.find(
    (manager) => normalizePersonId(manager.person_id) === normalizePersonId(value),
  );

  return (
    <Select
      value={current?.person_id ?? null}
      onValueChange={(root) => {
        if (root) setScope({ root: String(root) });
      }}
    >
      <SelectTrigger size="sm" aria-label="Team" className="w-56">
        <SelectValue>{current ? `${current.name}'s team` : "Team"}</SelectValue>
      </SelectTrigger>
      <SelectContent align="end">
        {managers.map((manager) => (
          <SelectItem
            key={manager.person_id}
            value={manager.person_id}
            style={{ paddingInlineStart: `${0.5 + manager.depth}rem` }}
          >
            {`${manager.name} (${manager.subtreeMemberCount})`}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

function MetricsTable({
  members,
  metricKeys,
  byKey,
}: {
  members: MembersGridMember[];
  metricKeys: readonly string[];
  byKey: Map<string, NormalizedMetricResult>;
}) {
  const [selection, setSelection] = useState<MemberSelection>({ kind: "all" });
  const [compare, setCompare] = useState(false);

  const shown = selectedMembers(members, selection);
  const shownIds = shown.map((member) => member.entityId);
  const columns = keysWithData(metricKeys, byKey, shownIds);
  const canCompare = selection.kind === "selected" && shown.length >= 2;
  const comparing = compare && canCompare;

  return (
    <section className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-3">
        <p className="me-auto text-sm text-muted-foreground">
          {`${shown.length} people · ${columns.length} metrics with data`}
        </p>
        <label className="flex items-center gap-2 text-sm select-none">
          <span aria-hidden>Compare</span>
          <Switch
            aria-label="Compare"
            checked={comparing}
            disabled={!canCompare}
            onCheckedChange={setCompare}
          />
        </label>
        <MemberFilter members={members} selection={selection} onChange={setSelection} />
      </div>
      <Card>
        <CardContent className="p-0">
          {shown.length === 0 ? (
            <p className="p-6 text-center text-sm text-muted-foreground">No people selected.</p>
          ) : comparing ? (
            <MetricsCompareTable people={shown} rows={compareRows(columns, byKey, shownIds)} />
          ) : (
            <MembersGrid
              members={shown}
              defaultSort="input"
              metricKeys={columns}
              byKey={byKey}
              caption="People × metrics"
            />
          )}
        </CardContent>
      </Card>
    </section>
  );
}
