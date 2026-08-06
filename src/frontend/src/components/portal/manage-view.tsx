import { useMemo } from "react";

import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type {
  MetricDefinitionSchemaStatus,
  MetricDefinition,
} from "@/api/metric-definitions-client";
import { useMetricDefinitions } from "@/queries/metric-definitions";
import { cn } from "@/lib/utils";

const STATUS_STYLE: Record<MetricDefinitionSchemaStatus, string> = {
  ok: "bg-success/15 text-success",
  error: "bg-destructive/15 text-destructive",
  unchecked: "bg-muted text-muted-foreground",
};

/**
 * Manage-zone surfaces backed by live data (Metric catalog, Data health).
 *
 * Both read the **unified** registry (`GET /v1/metric-definitions`) — the set
 * of metrics `/v1/metric-results` actually serves. The legacy
 * `/catalog/get_metrics` surface describes a disjoint, pre-catalog key
 * namespace (`*_bullet_rows.*`), so listing it here showed an admin a catalog
 * no portal surface reads (constructorfabric/insight#1988).
 */
export function ManageView({ item }: { item: string | null }) {
  if (item === "metric-catalog") return <MetricCatalogTable />;
  if (item === "data-health") return <DataHealth />;
  return (
    <div className="mx-auto w-full max-w-md p-8">
      <ComingSoon
        variant="card"
        state="empty"
        label="Admin surface — not wired yet"
      />
    </div>
  );
}

/** Flatten the prefix-grouped query result into one key-sorted list. */
function useFlatDefinitions() {
  const q = useMetricDefinitions();
  const metrics = useMemo<MetricDefinition[]>(
    () =>
      (q.data ?? [])
        .flatMap((g) => g.metrics)
        .sort((a, b) => a.metric_key.localeCompare(b.metric_key)),
    [q.data],
  );
  return { metrics, isLoading: q.isLoading, isError: q.isError, refetch: q.refetch };
}

const DIRECTION_LABEL: Record<MetricDefinition["direction"], string> = {
  higher_is_better: "higher is better",
  lower_is_better: "lower is better",
  neutral: "neutral",
};

function MetricCatalogTable() {
  const { metrics, isLoading, isError, refetch } = useFlatDefinitions();
  if (isLoading) return <CenteredSpinner className="min-h-[60vh]" />;
  if (isError)
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon variant="card" state="error" onRetry={() => refetch()} />
      </div>
    );

  return (
    <div className="flex flex-col gap-3 p-4 md:p-6">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">Metric catalog</h1>
        <p className="text-sm text-muted-foreground">
          {metrics.length} metrics · live from{" "}
          <code className="text-xs">/v1/metric-definitions</code>
        </p>
      </div>
      <div className="overflow-x-auto rounded-lg border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Metric key</TableHead>
              <TableHead>Label</TableHead>
              <TableHead>Unit</TableHead>
              <TableHead>Direction</TableHead>
              <TableHead>Dimensions</TableHead>
              <TableHead>Last observed</TableHead>
              <TableHead>Schema</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {metrics.map((m) => (
              <TableRow key={m.metric_key}>
                <TableCell className="font-mono text-xs">{m.metric_key}</TableCell>
                <TableCell>{m.short_label ?? m.label}</TableCell>
                <TableCell className="text-muted-foreground">
                  {m.unit || "—"}
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {DIRECTION_LABEL[m.direction]}
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {m.dimensions.length ? m.dimensions.join(" · ") : "—"}
                </TableCell>
                {/* A definition can exist with no observation for this tenant —
                    that is a data state, not an error. Say so plainly. */}
                <TableCell className="text-muted-foreground">
                  {m.last_observed_date ?? "no data yet"}
                </TableCell>
                <TableCell>
                  <Badge
                    variant="secondary"
                    className={cn("font-medium", STATUS_STYLE[m.schema_status])}
                  >
                    {m.schema_status}
                    {m.schema_error_code ? ` · ${m.schema_error_code}` : ""}
                  </Badge>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}

function DataHealth() {
  const { metrics, isLoading, isError, refetch } = useFlatDefinitions();
  if (isLoading) return <CenteredSpinner className="min-h-[60vh]" />;
  if (isError)
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon variant="card" state="error" onRetry={() => refetch()} />
      </div>
    );

  const counts: Record<MetricDefinitionSchemaStatus, number> = {
    ok: 0,
    error: 0,
    unchecked: 0,
  };
  for (const m of metrics) counts[m.schema_status] += 1;
  // A definition whose schema checks out can still have never produced a row
  // for this tenant — two separate questions, so show both answers.
  const noData = metrics.filter((m) => m.last_observed_date == null).length;

  return (
    <div className="flex flex-col gap-4 p-4 md:p-6">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">Data health</h1>
        <p className="text-sm text-muted-foreground">
          Schema-check status across {metrics.length} metrics
        </p>
      </div>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(11rem,1fr))] gap-3">
        {(["ok", "error", "unchecked"] as const).map((s) => (
          <div key={s} className="rounded-lg border bg-card p-4">
            <div className="text-3xl font-semibold tabular-nums">{counts[s]}</div>
            <div
              className={cn(
                "mt-1 inline-flex rounded-full px-2 py-0.5 text-xs font-medium",
                STATUS_STYLE[s],
              )}
            >
              {s}
            </div>
          </div>
        ))}
        <div className="rounded-lg border bg-card p-4">
          <div className="text-3xl font-semibold tabular-nums">{noData}</div>
          <div className="mt-1 inline-flex rounded-full bg-muted px-2 py-0.5 text-xs font-medium text-muted-foreground">
            no data yet
          </div>
        </div>
      </div>
    </div>
  );
}
