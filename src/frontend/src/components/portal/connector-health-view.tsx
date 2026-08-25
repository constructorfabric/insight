import { Fragment, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";

import type {
  ConnectorHealthRow,
  RunEvent,
} from "@/api/connector-health-client";
import { Badge } from "@/components/ui/badge";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  connectorStateLabel,
  formatAge,
  formatBytes,
  formatDelivery,
  formatDuration,
  stateCounts,
  triggerLabel,
  type ConnectorState,
  type ConnectorStateLabel,
} from "@/lib/portal/connector-health";
import { useConnectorHealth, useConnectorRuns } from "@/queries/connector-health";
import { TEXT_FIGURE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

const TONE_STYLE: Record<ConnectorStateLabel["tone"], string> = {
  critical: "bg-destructive/15 text-destructive",
  warning: "bg-warning/15 text-warning",
  ok: "bg-success/15 text-success",
  idle: "bg-muted text-muted-foreground",
};

/** Tiles lead with what needs acting on; a state nobody is in shows no tile. */
const TILE_ORDER: ConnectorState[] = [
  "misdelivered",
  "run_failed",
  "transform_failed",
  "sync_without_transform",
  "delivering",
  "never_ran",
  "not_configured",
];

const TILE_LABELS: Record<ConnectorState, string> = {
  misdelivered: "nothing landed",
  run_failed: "run failed",
  transform_failed: "transform failed",
  sync_without_transform: "sync without transform",
  delivering: "delivering",
  never_ran: "never ran",
  not_configured: "not configured",
};

export function ConnectorHealthView() {
  const { data, isLoading, isError, refetch } = useConnectorHealth();
  const [expanded, setExpanded] = useState<string | null>(null);

  if (isLoading) return <CenteredSpinner className="min-h-[60vh]" />;
  if (isError) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon variant="card" state="error" onRetry={() => refetch()} />
      </div>
    );
  }

  const rows = data?.connectors ?? [];
  const counts = stateCounts(rows);

  return (
    <div className="flex flex-col gap-4 p-4 md:p-6">
      <header>
        <h1 className="text-lg font-semibold tracking-tight">
          Connector health
        </h1>
        <p className="text-sm text-muted-foreground">
          {rows.length} connectors ·{" "}
          {data?.history_available
            ? `swept ${formatAge(data.as_of)}`
            : "no run history recorded yet"}
        </p>
      </header>

      {/* The page must not read as health when nothing has recorded anything. */}
      {!data?.history_available && (
        <p className="rounded-lg border bg-card p-4 text-sm text-muted-foreground">
          Nothing has recorded an ingestion run on this installation yet. What
          follows is what storage holds, not a statement about delivery.
        </p>
      )}

      <div className="grid grid-cols-[repeat(auto-fit,minmax(10rem,1fr))] gap-3">
        {TILE_ORDER.filter((state) => counts.get(state)).map((state) => (
          <div key={state} className="rounded-lg border bg-card p-4">
            <div className={TEXT_FIGURE}>{counts.get(state)}</div>
            <div className="mt-1 text-xs font-medium text-muted-foreground">
              {TILE_LABELS[state]}
            </div>
          </div>
        ))}
      </div>

      <div className="overflow-x-auto rounded-lg border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-8" />
              <TableHead>Connector</TableHead>
              <TableHead>State</TableHead>
              <TableHead>Last run</TableHead>
              <TableHead>Recorded / landed</TableHead>
              <TableHead>Streams</TableHead>
              <TableHead className="text-right">Rows stored</TableHead>
              <TableHead className="text-right">Size</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <Fragment key={row.connector}>
                <ConnectorSummaryRow
                  row={row}
                  isExpanded={expanded === row.connector}
                  onToggle={() =>
                    setExpanded(
                      expanded === row.connector ? null : row.connector
                    )
                  }
                />
                {expanded === row.connector && (
                  <TableRow>
                    <TableCell colSpan={8} className="bg-muted/40 p-0">
                      <ConnectorDetail row={row} />
                    </TableCell>
                  </TableRow>
                )}
              </Fragment>
            ))}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}

function ConnectorSummaryRow({
  row,
  isExpanded,
  onToggle,
}: {
  row: ConnectorHealthRow;
  isExpanded: boolean;
  onToggle: () => void;
}) {
  const state = connectorStateLabel(row);
  const Chevron = isExpanded ? ChevronDown : ChevronRight;
  const trigger = triggerLabel(row.last_sync?.trigger ?? null);

  return (
    <TableRow
      className="cursor-pointer"
      onClick={onToggle}
      aria-expanded={isExpanded}
    >
      <TableCell>
        <Chevron className="size-4 text-muted-foreground" aria-hidden />
      </TableCell>
      <TableCell className="font-medium">{row.connector}</TableCell>
      <TableCell>
        <Badge
          variant="secondary"
          className={cn("font-medium", TONE_STYLE[state.tone])}
        >
          {state.label}
        </Badge>
      </TableCell>
      <TableCell className="text-muted-foreground">
        {row.last_run ? (
          <>
            {formatAge(row.last_run.started_at)}
            {row.last_run.step && (
              <span className="block text-xs">
                stopped at {row.last_run.step}
              </span>
            )}
          </>
        ) : (
          "—"
        )}
      </TableCell>
      <TableCell className="text-muted-foreground">
        {row.last_sync ? (
          <>
            {formatDelivery(
              row.last_sync.records_moved,
              row.last_sync.rows_landed
            )}
            {trigger && <span className="block text-xs">{trigger}</span>}
          </>
        ) : (
          "—"
        )}
      </TableCell>
      <TableCell className="text-muted-foreground">
        {row.storage
          ? `${row.storage.streams_with_data} of ${row.storage.streams}`
          : "—"}
      </TableCell>
      <TableCell className="text-right text-muted-foreground tabular-nums">
        {row.storage ? row.storage.physical_rows.toLocaleString("en-US") : "—"}
      </TableCell>
      <TableCell className="text-right text-muted-foreground tabular-nums">
        {row.storage ? formatBytes(row.storage.bytes_on_disk) : "—"}
      </TableCell>
    </TableRow>
  );
}

function ConnectorDetail({ row }: { row: ConnectorHealthRow }) {
  return (
    <div className="grid gap-6 p-4 md:grid-cols-2">
      <section className="grid gap-2">
        <h2 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Streams
        </h2>
        {row.streams.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No stream observation recorded yet.
          </p>
        ) : (
          <ul className="grid gap-1 text-sm">
            {row.streams.map((stream) => (
              <li
                key={stream.stream}
                className="flex justify-between gap-4 border-b py-1 last:border-b-0"
              >
                <span className="font-mono text-xs">{stream.stream}</span>
                <span className="tabular-nums text-muted-foreground">
                  {stream.physical_rows === 0
                    ? "empty"
                    : stream.physical_rows.toLocaleString("en-US")}
                </span>
              </li>
            ))}
          </ul>
        )}
        {row.storage && (
          <p className="text-xs text-muted-foreground">
            Physical rows, observed {formatAge(row.storage.observed_at)}. On a
            deduplicating store this sizes a stream; it does not count entities.
          </p>
        )}
      </section>

      <RunHistory connector={row.connector} />
    </div>
  );
}

function RunHistory({ connector }: { connector: string }) {
  const { data, isLoading, isError } = useConnectorRuns(connector);

  return (
    <section className="grid gap-2">
      <h2 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        Recent runs
      </h2>
      {isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
      {isError && (
        <p className="text-sm text-muted-foreground">
          Could not read the run history.
        </p>
      )}
      {data && data.runs.length === 0 && (
        <p className="text-sm text-muted-foreground">
          No runs recorded for this connector.
        </p>
      )}
      {data && data.runs.length > 0 && (
        <ul className="grid gap-1 text-sm">
          {data.runs.slice(0, 12).map((event, index) => (
            <li
              key={`${event.event}-${event.started_at}-${index}`}
              className="flex justify-between gap-4 border-b py-1 last:border-b-0"
            >
              <span className="font-mono text-xs">{event.event}</span>
              <span className="text-muted-foreground">
                {describeEvent(event)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function describeEvent(event: RunEvent): string {
  const when = formatAge(event.started_at);
  const took = event.duration_ms > 0 ? ` · ${formatDuration(event.duration_ms)}` : "";
  const step = event.step ? ` at ${event.step}` : "";
  return `${event.status}${step} · ${when}${took}`;
}
