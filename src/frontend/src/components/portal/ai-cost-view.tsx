import { useMemo } from "react";

import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { PANE_ITEM_COMING_SOON } from "@/lib/portal/aicost-configs";
import { orgScopeGate } from "@/components/portal/org-scope-gate";
import { personDisplayName } from "@/lib/identities/person-display";
import { MembersGrid } from "@/components/widgets/dashboard/members-grid";
import { Card, CardContent } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { usePortalPeriod } from "@/hooks/use-portal-period";
import { formatMetricValue } from "@/lib/format";
import { GROUPS } from "@/lib/insight/groups";
import {
  availableSlices,
  cohortKey,
  collectRosterAttrs,
  PLANNED_SLICES,
  type SliceDim,
} from "@/lib/insight/slices";
import { quantile, withinCohortPeer } from "@/lib/insight/within-team-peer";
import {
  forEntity,
  type MetricCollectionConfig,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import { normalizePersonId } from "@/lib/metrics/entity";
import {
  usePortalSlice,
} from "@/lib/portal/portal-nav";
import type { TeamMember } from "@/types/insight";
import { useCohortLabel } from "@/lib/portal/use-cohort-label";
import { useOrgScope } from "@/lib/portal/use-org-scope";
import { useMemberGridData } from "@/queries/member-grid";
import { useMetricCollection } from "@/queries/metric-results";
import { TEXT_FIGURE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

const EMPTY_COLLECTION: MetricCollectionConfig = { metrics: [] };
const COST_KEY = "ai.cost";
const ACTUAL_COST_KEY = "ai.extra_usage_cost";
const LINES_KEY = "ai.accepted_lines";
const DAYS_KEY = "ai.active_days";
/** Grid columns for the cost-leaders scan. */
const GRID_KEYS = [
  COST_KEY,
  ACTUAL_COST_KEY,
  DAYS_KEY,
  LINES_KEY,
  "ai.dev_conversations",
];

const TOOL_LABEL: Record<string, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
  chatgpt: "ChatGPT",
};

interface ToolRow {
  tool: string;
  users: number;
  lines: number;
  cost: number;
  costTracked: boolean;
  actual: number;
  actualTracked: boolean;
}

/** Sum a breakdown metric across members, grouped by the `tool` dimension. */
function aggregateByTool(
  result: NormalizedMetricResult | undefined,
  memberIds: readonly string[],
): Map<string, { sum: number; users: Set<string> }> {
  const out = new Map<string, { sum: number; users: Set<string> }>();
  if (!result) return out;
  for (const id of memberIds) {
    for (const row of forEntity(result, id).breakdown) {
      const tool = row.dimensions.find((d) => d.key === "tool")?.value;
      if (!tool || row.value == null) continue;
      const bucket = out.get(tool) ?? { sum: 0, users: new Set<string>() };
      bucket.sum += row.value;
      if (row.value > 0) bucket.users.add(id);
      out.set(tool, bucket);
    }
  }
  return out;
}

const PLANNED_KEYS = new Set(PLANNED_SLICES.map((d) => d.key));

/**
 * AI & Cost — org-level adoption + spend across the active org scope (set in
 * the topbar). Shows what's actually ingested; everything unavailable is an
 * explicit ComingSoon rather than a fabricated or zero-filled panel.
 *
 * Honest data caveats surfaced in the UI:
 *  - only Claude Code is usage-metered → the cost total is Claude-only;
 *  - ChatGPT/Codex report usage but no per-user cost (subscription / token
 *    billing isn't ingested), so their cost reads "not tracked", never $0;
 *  - actual cost is a seat-month fact, so it does not narrow to a period
 *    shorter than a month while the potential figure beside it does.
 */
export function AiCostView({ item }: { item: string | null }) {
  const cohortLabel = useCohortLabel();
  const { period, dateRange } = usePortalPeriod();

  const orgScope = useOrgScope();
  const { pivot, roster } = orgScope;

  // The roster IS the member list: identity owns who is on the team and
  // every metric for them comes from `/v1/metric-results`. There is no second
  // source to reconcile — the legacy per-member batch this used to call was
  // removed upstream with the rest of the old metric UI.
  const members = useMemo<TeamMember[]>(
    () =>
      (roster ?? []).map((entry) => ({
        person_id: entry.person_id,
        name: personDisplayName(entry),
      })),
    [roster],
  );
  const memberIds = useMemo(
    () => members.map((m) => normalizePersonId(m.person_id)),
    [members],
  );

  const aiGroup = useMemo(
    () => GROUPS.find((g) => g.id === "ai_adoption") ?? null,
    [],
  );
  const gridCollection = useMemo<MetricCollectionConfig>(
    () => ({
      metrics: (aiGroup?.collection.metrics ?? []).filter((m) =>
        GRID_KEYS.includes(m.key),
      ),
    }),
    [aiGroup],
  );
  const toolCollection = useMemo<MetricCollectionConfig>(
    () => ({
      metrics: [COST_KEY, ACTUAL_COST_KEY, LINES_KEY].map((key) => ({
        key,
        views: [{ view: "breakdown" as const, dimensions: ["tool"] }],
      })),
    }),
    [],
  );

  const grid = useMemberGridData(
    memberIds.length ? gridCollection : EMPTY_COLLECTION,
    { type: "person", ids: memberIds },
    dateRange,
    period,
  );
  const toolData = useMetricCollection(
    memberIds.length ? toolCollection : EMPTY_COLLECTION,
    { type: "person", ids: memberIds },
    dateRange,
  );

  const teamName = orgScope.label;

  const sum = (key: string) => {
    const r = grid.byKey.get(key);
    if (!r) return 0;
    return memberIds.reduce((acc, id) => {
      const v = forEntity(r, id).value;
      return acc + (v != null && Number.isFinite(v) ? v : 0);
    }, 0);
  };
  const activeUsers = useMemo(() => {
    const r = grid.byKey.get(DAYS_KEY);
    if (!r) return 0;
    return memberIds.filter((id) => (forEntity(r, id).value ?? 0) > 0).length;
  }, [grid.byKey, memberIds]);

  const toolRows = useMemo<ToolRow[]>(() => {
    const cost = aggregateByTool(toolData.byKey.get(COST_KEY), memberIds);
    // Claude Team bills the seat under `claude`; the coding usage that seat
    // paid for arrives as `claude_code`. One card, not two.
    const actual = new Map(
      [...aggregateByTool(toolData.byKey.get(ACTUAL_COST_KEY), memberIds)].map(
        ([tool, sums]) => [tool === "claude" ? "claude_code" : tool, sums] as const,
      ),
    );
    const lines = aggregateByTool(toolData.byKey.get(LINES_KEY), memberIds);
    const tools = new Set([...cost.keys(), ...actual.keys(), ...lines.keys()]);
    return [...tools]
      .map((tool) => {
        const c = cost.get(tool);
        const a = actual.get(tool);
        const l = lines.get(tool);
        return {
          tool,
          users: (l?.users.size ?? 0) || (c?.users.size ?? 0),
          lines: l?.sum ?? 0,
          cost: c?.sum ?? 0,
          costTracked: (c?.sum ?? 0) > 0,
          actual: a?.sum ?? 0,
          // Only vendors that bill above a seat fee report this, so a tool
          // without it says nothing rather than $0 — the rule the potential
          // figure already follows.
          actualTracked: (a?.sum ?? 0) > 0,
        };
      })
      .sort((a, b) => b.lines - a.lines);
  }, [toolData.byKey, memberIds]);

  // Data-driven slice options + the active slice's cohort accessor. The slice
  // is global (portal-store) so it re-cohorts every view at once.
  const attrByEntity = useMemo(
    () => collectRosterAttrs(pivot, normalizePersonId),
    [pivot],
  );
  const sliceDims = useMemo<SliceDim[]>(
    () => availableSlices(attrByEntity.values()),
    [attrByEntity],
  );
  const slice = usePortalSlice();
  const cohortOf = useMemo(
    () => (id: string) => cohortKey(attrByEntity.get(id), slice),
    [attrByEntity, slice],
  );

  // Grid heat / attention rank each person within their active-slice cohort
  // (whole roster when no slice is set).
  const heatByKey = useMemo(() => {
    const m = new Map<string, NormalizedMetricResult>();
    for (const k of GRID_KEYS) {
      const r = grid.byKey.get(k);
      if (r) m.set(k, withinCohortPeer(r, memberIds, cohortOf));
    }
    return m;
  }, [grid.byKey, memberIds, cohortOf]);

  // "By unit" groups the roster by the active slice attribute.
  const unitRows = useMemo(() => {
    if (!slice || PLANNED_KEYS.has(slice)) return [];
    const costR = grid.byKey.get(COST_KEY);
    const actualR = grid.byKey.get(ACTUAL_COST_KEY);
    const linesR = grid.byKey.get(LINES_KEY);
    const daysR = grid.byKey.get(DAYS_KEY);
    const val = (r: NormalizedMetricResult | undefined, id: string) =>
      r ? (forEntity(r, id).value ?? 0) : 0;
    const map = new Map<
      string,
      {
        unit: string;
        people: number;
        active: number;
        cost: number;
        actual: number;
        lines: number;
      }
    >();
    for (const id of memberIds) {
      const unit = attrByEntity.get(id)?.[slice]?.value ?? "—";
      const b =
        map.get(unit) ??
        { unit, people: 0, active: 0, cost: 0, actual: 0, lines: 0 };
      b.people += 1;
      if (val(daysR, id) > 0) b.active += 1;
      b.cost += val(costR, id);
      b.actual += val(actualR, id);
      b.lines += val(linesR, id);
      map.set(unit, b);
    }
    return [...map.values()].sort((a, b) => b.cost - a.cost || b.lines - a.lines);
  }, [grid.byKey, memberIds, attrByEntity, slice]);

  // Adoption funnel from active-days: org → any use → active → heavy. Stage
  // cuts are data-relative (median / top-quartile among AI users), so they hold
  // regardless of the selected period length.
  const funnel = useMemo(() => {
    const daysR = grid.byKey.get(DAYS_KEY);
    const days = memberIds.map((id) => (daysR ? (forEntity(daysR, id).value ?? 0) : 0));
    const users = days.filter((d) => d > 0).sort((a, b) => a - b);
    // Rounded ONCE and reused for both the label and the comparison, so the
    // stage count always matches the threshold the label promises.
    const med = users.length ? Math.round(quantile(users, 0.5)) : 0;
    const p75 = users.length ? Math.round(quantile(users, 0.75)) : 0;
    return [
      { label: "In org", n: memberIds.length },
      { label: "Used AI (≥1 day)", n: users.length },
      { label: `Active (≥${med} days · median)`, n: days.filter((d) => d > 0 && d >= med).length },
      { label: `Heavy (≥${p75} days · top quartile)`, n: days.filter((d) => d > 0 && d >= p75).length },
    ];
  }, [grid.byKey, memberIds]);

  if (item && PANE_ITEM_COMING_SOON[item])
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon variant="card" state="empty" label={PANE_ITEM_COMING_SOON[item]} />
      </div>
    );

  const gate = orgScopeGate({
    viewerLoading: orgScope.isLoading,
    viewerError: orgScope.isError,
    membersLoading: false,
    membersError: false,
    memberCount: members.length,
    gridPending: grid.isPending,
    gridError: grid.isError,
    emptyLabel: "No people in the current scope. Pick a different scope at the top of the page.",
    onRetry: () => {
      orgScope.refetch();
      grid.refetch();
    },
  });
  if (gate) return gate;

  const costR = grid.byKey.get(COST_KEY);
  const actualR = grid.byKey.get(ACTUAL_COST_KEY);
  const linesR = grid.byKey.get(LINES_KEY);
  const totalCost = sum(COST_KEY);
  const totalActual = sum(ACTUAL_COST_KEY);
  const totalLines = sum(LINES_KEY);
  const adoptionPct = members.length
    ? Math.round((activeUsers / members.length) * 100)
    : 0;
  const avgCost = activeUsers ? totalCost / activeUsers : 0;
  const avgActual = activeUsers ? totalActual / activeUsers : 0;
  const actualMoney = (value: number) =>
    formatMetricValue(
      value,
      actualR?.format ?? "currency",
      actualR?.unit ?? "USD",
    );
  // Absent, not zero: `sum` folds nulls to 0, so a $0 total cannot be told from
  // no seat data — and printing $0 would claim nothing was billed.
  const hasActual = !!actualR && totalActual > 0;

  const shownGridKeys = GRID_KEYS.filter((k) => {
    const r = grid.byKey.get(k);
    return (
      !!r &&
      memberIds.some((id) => {
        const v = forEntity(r, id).value;
        return v != null && Number.isFinite(v);
      })
    );
  });

  return (
    <div className="flex flex-col gap-6 p-4 md:p-6">
      <div>
        <h1 className="text-lg font-semibold tracking-tight">AI &amp; Cost</h1>
        <p className="text-sm text-muted-foreground">
          {teamName ? `${teamName}'s org` : "Org"} · {orgScope.count} people
        </p>
      </div>

      {/* Headline */}
      <div className="grid grid-cols-[repeat(auto-fit,minmax(11rem,1fr))] gap-3">
        <Tile
          label="AI potential usage cost"
          value={formatMetricValue(totalCost, costR?.format ?? "currency", costR?.unit ?? "USD")}
          note={hasActual ? `actual cost ${actualMoney(totalActual)}` : null}
          sub="Claude Code only"
        />
        <Tile label="Active AI users" value={String(activeUsers)} sub={`${adoptionPct}% of ${members.length}`} />
        <Tile
          label="AI-accepted lines"
          value={formatMetricValue(totalLines, linesR?.format ?? "integer", linesR?.unit ?? null)}
          sub="org total"
        />
        <Tile
          label="Avg potential cost / active user"
          value={formatMetricValue(
            avgCost,
            costR?.format ?? "currency",
            costR?.unit ?? null,
          )}
          note={
            hasActual
              ? `avg actual ${actualMoney(avgActual)} / active user`
              : null
          }
          sub="Claude Code"
        />
      </div>

      {item === "adoption-funnel" ? (
        <FunnelSection funnel={funnel} />
      ) : item === "by-unit-role" ? (
        <UnitSection
          rows={unitRows}
          costR={costR}
          actualR={actualR}
          linesR={linesR}
          dim={[...sliceDims, ...PLANNED_SLICES].find((d) => d.key === slice) ?? null}
        />
      ) : (
        <>
      {/* By tool */}
      <section className="flex flex-col gap-3">
        <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
          By tool
        </p>
        {toolData.isError ? (
          // A failed breakdown is not an empty one: reporting "nothing for this
          // period" would hide exactly the failure the scope gate exists to
          // surface, and the reader would take a broken request for a fact
          // about their org.
          <ComingSoon state="error" label="Unable to load the per-tool breakdown" />
        ) : toolData.isPending ? (
          <CenteredSpinner className="min-h-32" />
        ) : toolRows.length ? (
          <div className="grid grid-cols-[repeat(auto-fit,minmax(13rem,1fr))] gap-3">
            {toolRows.map((t) => (
              <Card key={t.tool}>
                <CardContent className="flex flex-col gap-1 p-4">
                  <div className="text-sm font-semibold">
                    {TOOL_LABEL[t.tool] ?? t.tool}
                  </div>
                  <div className={TEXT_FIGURE}>
                    {t.costTracked
                      ? formatMetricValue(t.cost, "currency", "USD")
                      : "—"}
                  </div>
                  <div className="text-xs text-muted-foreground">
                    {t.costTracked ? "potential cost" : "potential cost not tracked"}
                  </div>
                  <div className="mt-1 text-xs text-muted-foreground">
                    {t.actualTracked
                      ? `actual cost ${formatMetricValue(t.actual, "currency", "USD")} · `
                      : ""}
                    {t.users} users · {formatMetricValue(t.lines, "integer", null)} lines
                  </div>
                </CardContent>
              </Card>
            ))}
          </div>
        ) : (
          <ComingSoon variant="card" state="empty" label="No breakdown by tool for this period." />
        )}
        <p className="text-xs text-muted-foreground">
          Only Claude Code is usage-metered. ChatGPT (per-seat subscription) and
          Codex (token-based) report usage but no per-user cost yet — shown as
          “not tracked”, not $0. Actual cost is a monthly figure, so a period
          shorter than a month shows the whole month’s billed amount.
        </p>
      </section>

      {/* Cost leaders */}
      <section className="flex flex-col gap-3">
        <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
          Cost &amp; usage by person
        </p>
        <Card>
          <CardContent className="p-0">
            <MembersGrid
              members={members.map((m) => ({
                entityId: normalizePersonId(m.person_id),
                displayName: m.name,
                personId: m.person_id,
              }))}
              metricKeys={shownGridKeys}
              byKey={heatByKey}
              previousByKey={grid.previousByKey}
              caption={`${teamName} — AI usage & cost by person`}
              cohortLabel={cohortLabel}
            />
          </CardContent>
        </Card>
      </section>
        </>
      )}
    </div>
  );
}

/** Adoption funnel — successive stages as proportional bars. */
function FunnelSection({ funnel }: { funnel: { label: string; n: number }[] }) {
  const top = funnel[0]?.n || 1;
  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        Adoption funnel
      </p>
      <Card>
        <CardContent className="flex flex-col gap-2 p-4">
          {funnel.map((s) => {
            const pct = Math.round((s.n / top) * 100);
            return (
              <div key={s.label} className="flex items-center gap-3">
                <div className="w-56 shrink-0 text-sm">{s.label}</div>
                <div className="relative h-7 flex-1 overflow-hidden rounded bg-muted">
                  <div
                    className="h-full rounded bg-primary/25"
                    style={{ width: `${pct}%` }}
                  />
                  <span className="absolute inset-y-0 left-2 flex items-center text-xs font-medium tabular-nums">
                    {s.n} · {pct}%
                  </span>
                </div>
              </div>
            );
          })}
        </CardContent>
      </Card>
      <p className="text-xs text-muted-foreground">
        Stages from AI active-days; “active”/“heavy” cuts are the median and
        top-quartile day counts among people who used AI this period.
      </p>
    </section>
  );
}

/** AI cost + adoption grouped by the active slice (driven by the header SliceSelect). */
function UnitSection({
  rows,
  costR,
  actualR,
  linesR,
  dim,
}: {
  rows: {
    unit: string;
    people: number;
    active: number;
    cost: number;
    actual: number;
    lines: number;
  }[];
  costR: NormalizedMetricResult | undefined;
  actualR: NormalizedMetricResult | undefined;
  linesR: NormalizedMetricResult | undefined;
  dim: SliceDim | null;
}) {
  const dimLabel = dim?.label ?? "Unit";
  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {dim ? `By ${dimLabel.toLowerCase()}` : "By unit"}
      </p>
      {!dim ? (
        <ComingSoon
          variant="card"
          state="empty"
          label="Pick a slice in the header (Division / Department / Title / Manager) to group the org."
        />
      ) : dim.planned ? (
        <ComingSoon
          variant="card"
          state="empty"
          label="Functional teams — derived team detection (co-commit clustering) isn't built yet; only reporting-line slices (division / department / manager) and title are available."
        />
      ) : (
      <div className="overflow-x-auto rounded-lg border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{dimLabel}</TableHead>
              <TableHead className="text-right">People</TableHead>
              <TableHead className="text-right">AI users</TableHead>
              <TableHead className="text-right">Potential cost</TableHead>
              <TableHead className="text-right">Actual cost</TableHead>
              <TableHead className="text-right">Accepted lines</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((r) => (
              <TableRow key={r.unit}>
                <TableCell className="font-medium">{r.unit}</TableCell>
                <TableCell className="text-right tabular-nums text-muted-foreground">
                  {r.people}
                </TableCell>
                <TableCell className="text-right tabular-nums text-muted-foreground">
                  {r.active}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {formatMetricValue(r.cost, costR?.format ?? "currency", costR?.unit ?? "USD")}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {actualR
                    ? formatMetricValue(
                        r.actual,
                        actualR.format,
                        actualR.unit ?? "USD",
                      )
                    : "—"}
                </TableCell>
                <TableCell className="text-right tabular-nums text-muted-foreground">
                  {formatMetricValue(r.lines, linesR?.format ?? "integer", linesR?.unit ?? null)}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
      )}
      {dim && !dim.planned ? (
        <p className="text-xs text-muted-foreground">
          Costs are Claude Code only (the usage-metered tool); actual cost is a
          monthly figure.
        </p>
      ) : null}
    </section>
  );
}

function Tile({
  label,
  value,
  note,
  sub,
}: {
  label: string;
  value: string;
  /** A second figure held beside `value`, never added to it. */
  note?: string | null;
  sub: string;
}) {
  return (
    <Card>
      <CardContent className="p-4">
        <div className="text-xs font-medium text-muted-foreground">{label}</div>
        <div className={cn("mt-1", TEXT_FIGURE)}>{value}</div>
        {note ? (
          <div className="text-xs font-medium tabular-nums text-muted-foreground">
            {note}
          </div>
        ) : null}
        <div className="text-xs text-muted-foreground">{sub}</div>
      </CardContent>
    </Card>
  );
}
