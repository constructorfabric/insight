import { useMemo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

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
import { IdentitiesView } from "@/components/portal/identities-view";
import { PlatformUsage } from "@/components/portal/platform-usage";
import { useIsAdmin } from "@/queries/identity-me";
import { useMetricDefinitions } from "@/queries/metric-definitions";
import { useConnectorHealth } from "@/queries/connector-health";
import {
  connectorState,
  elapsedSince,
  orderByAttention,
} from "@/lib/portal/connector-health";
import { seriesColors } from "@/lib/series-colors";
import { WhatsNewBody } from "@/screens/whats-new";
import { TEXT_FIGURE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

const CONNECTOR_STATE_STYLE: Record<string, string> = {
  delivering: "bg-success/15 text-success",
  partial: "bg-warning/20 text-warning-foreground",
  never: "bg-destructive/15 text-destructive",
};

const STATUS_STYLE: Record<MetricDefinitionSchemaStatus, string> = {
  ok: "bg-success/15 text-success",
  error: "bg-destructive/15 text-destructive",
  unchecked: "bg-muted text-muted-foreground",
};

/**
 * Manage-zone surfaces (Metric catalog, Data health, Identities, What's new).
 *
 * The two catalog surfaces read the **unified** registry
 * (`GET /v1/metric-definitions`) — the set
 * of metrics `/v1/metric-results` actually serves. The legacy
 * `/catalog/get_metrics` surface describes a disjoint, pre-catalog key
 * namespace (`*_bullet_rows.*`), so listing it here showed an admin a catalog
 * no portal surface reads (constructorfabric/insight#1988).
 */
export function ManageView({ item }: { item: string | null }) {
  if (item === "metric-catalog") return <MetricCatalogTable />;
  if (item === "data-health")
    return (
      <AdminGate>
        <DataHealth />
      </AdminGate>
    );
  if (item === "identities")
    return (
      <AdminGate>
        <IdentitiesView />
      </AdminGate>
    );
  if (item === "platform-usage")
    return (
      <AdminGate>
        <PlatformUsage />
      </AdminGate>
    );
  if (item === "whats-new") return <WhatsNewBody />;
  return (
    <div className="mx-auto w-full max-w-md p-8">
      <ComingSoon variant="card" state="empty" label="Not built yet" />
    </div>
  );
}

/**
 * The role gate in front of the identity-resolution console. Bookmarks and
 * pasted URLs land here directly (the nav hides the item, the URL does not),
 * so a non-admin gets an explicit refusal rather than a broken or empty
 * screen — and never a flash of the console while the check is in flight.
 * A FAILED check is a third state: still no console (fail closed), but the
 * copy says "could not verify" with a retry — telling a real admin to go ask
 * for a role they already hold would send them chasing a grant that fixes
 * nothing.
 */
function AdminGate({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const { isAdmin, isPending, isError, retry } = useIsAdmin();
  if (isPending) return <CenteredSpinner />;
  if (isError) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon
          variant="card"
          state="error"
          label={t("identities.gate.unverified")}
          onRetry={retry}
        />
      </div>
    );
  }
  if (!isAdmin) {
    return (
      <div className="mx-auto w-full max-w-md p-8" role="alert">
        <div className="rounded-lg border p-6 text-center">
          <p className="text-sm font-semibold">{t("identities.gate.title")}</p>
          <p className="mt-2 text-sm text-muted-foreground">
            {t("identities.gate.description")}
          </p>
        </div>
      </div>
    );
  }
  return children;
}

/** Flatten the prefix-grouped query result into one key-sorted list. */
function useFlatDefinitions() {
  const q = useMetricDefinitions();
  const metrics = useMemo<MetricDefinition[]>(
    () =>
      (q.data ?? [])
        .flatMap((g) => g.metrics)
        .sort((a, b) => a.metric_key.localeCompare(b.metric_key)),
    [q.data]
  );
  return {
    metrics,
    isLoading: q.isLoading,
    isError: q.isError,
    refetch: q.refetch,
  };
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
        <h1 className="text-lg font-semibold tracking-tight">Metric catalog</h1>
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
                <TableCell className="font-mono text-xs">
                  {m.metric_key}
                </TableCell>
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
  const { t } = useTranslation();

  return (
    <div className="flex flex-col gap-6 p-4 md:p-6">
      <h1 className="text-lg font-semibold tracking-tight">
        {t("data_health.heading")}
      </h1>

      <ConnectorDelivery />
    </div>
  );
}

function ConnectorDelivery() {
  const { t } = useTranslation();
  const { data, isLoading, isError, refetch } = useConnectorHealth();

  if (isLoading) return <CenteredSpinner className="min-h-[12rem]" />;
  if (isError || data == null)
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon variant="card" state="error" onRetry={() => refetch()} />
      </div>
    );

  const asOf = new Date(data.as_of);
  const connectors = orderByAttention(data.connectors);
  const states = connectors.map((c) => connectorState(c));
  const count = (state: string) => states.filter((s) => s === state).length;
  const dots = seriesColors(connectors.map((c) => c.connector));

  const tiles = [
    { key: "delivering", value: count("delivering"), tone: "text-success" },
    {
      key: "partial",
      value: count("partial"),
      tone: count("partial") > 0 ? "text-warning-foreground" : "",
    },
    {
      key: "never",
      value: count("never"),
      tone: count("never") > 0 ? "text-destructive" : "",
    },
  ] as const;

  if (connectors.length === 0)
    return (
      <div className="flex flex-col gap-4">
        <h2 className="text-sm font-medium text-muted-foreground">
          {t("connector_health.heading")}
        </h2>
        <div className="rounded-lg border bg-card p-6 text-sm text-muted-foreground">
          {t("connector_health.empty")}
        </div>
      </div>
    );

  return (
    <div className="flex flex-col gap-4">
      <h2 className="text-sm font-medium text-muted-foreground">
        {t("connector_health.heading")}
      </h2>

      <div
        data-connector-tiles
        className="grid grid-cols-[repeat(auto-fit,minmax(9rem,1fr))] gap-3"
      >
        {tiles.map((tile) => (
          <div key={tile.key} className="rounded-lg border bg-card p-4">
            <div className={cn(TEXT_FIGURE, tile.tone)}>{tile.value}</div>
            <div className="mt-1 text-xs text-muted-foreground">
              {t(`connector_health.tiles.${tile.key}`)}
            </div>
          </div>
        ))}
      </div>

      <div className="overflow-x-auto rounded-lg border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("connector_health.columns.connector")}</TableHead>
              <TableHead>{t("connector_health.columns.last_data")}</TableHead>
              <TableHead>
                {t("connector_health.columns.streams_with_data")}
              </TableHead>
              <TableHead className="text-end">
                {t("connector_health.columns.rows")}
              </TableHead>
              <TableHead>{t("connector_health.columns.state")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {connectors.map((c) => {
              const elapsed = elapsedSince(c.last_write, asOf);
              const state = connectorState(c);
              return (
                <TableRow key={c.namespace}>
                  <TableCell className="font-medium">
                    <span className="flex items-center gap-2.5">
                      <span
                        aria-hidden
                        data-connector-dot
                        className="size-2 shrink-0 rounded-full"
                        style={{ background: dots[c.connector] }}
                      />
                      {c.connector}
                    </span>
                  </TableCell>
                  <TableCell
                    className={cn(
                      "tabular-nums",
                      state === "never" && "text-muted-foreground",
                    )}
                  >
                    {elapsed.kind === "never" || elapsed.kind === "unknown"
                      ? t(`connector_health.elapsed.${elapsed.kind}`)
                      : t(`connector_health.elapsed.${elapsed.kind}`, {
                          count: elapsed.value,
                        })}
                  </TableCell>
                  <TableCell className="tabular-nums">
                    {t("connector_health.streams_fraction", {
                      withData: c.streams_with_data,
                      total: c.streams,
                    })}
                  </TableCell>
                  <TableCell className="text-end tabular-nums">
                    {c.rows.toLocaleString()}
                  </TableCell>
                  <TableCell>
                    <Badge
                      variant="secondary"
                      className={cn("font-medium", CONNECTOR_STATE_STYLE[state])}
                    >
                      {t(`connector_health.state.${state}`)}
                    </Badge>
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}
