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
  activatesRow,
  activatesRowByKey,
} from "@/lib/identities/row-activation";
import {
  connectorStateLabel,
  formatAge,
  formatBytes,
  formatDelivery,
  formatDuration,
  stateCounts,
  stateTileLabel,
  triggerLabel,
  STATE_ORDER,
  type ConnectorStateLabel,
} from "@/lib/portal/connector-health";
import { useConnectorHealth, useConnectorRuns } from "@/queries/connector-health";
import { TEXT_FIGURE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/** How many recorded events the expansion lists before saying there are more. */
const HISTORY_SHOWN = 12;

const TONE_STYLE: Record<ConnectorStateLabel["tone"], string> = {
  critical: "bg-destructive/15 text-destructive",
  warning: "bg-warning/15 text-warning",
  ok: "bg-success/15 text-success",
  idle: "bg-muted text-muted-foreground",
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
          {/* The recorded marker, never the reader's own clock — that would say
              "just now" however long ago the controller last ran. */}
          {data?.swept_at
            ? `last swept ${formatAge(data.swept_at)}`
            : "never swept"}
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
        {STATE_ORDER.filter((state) => counts.get(state)).map((state) => (
          <div key={state} className="rounded-lg border bg-card p-4">
            <div className={TEXT_FIGURE}>{counts.get(state)}</div>
            <div className="mt-1 text-xs font-medium text-muted-foreground">
              {stateTileLabel(state)}
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
              <TableHead>Managed</TableHead>
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
                    <TableCell colSpan={9} className="bg-muted/40 p-0">
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
      role="button"
      tabIndex={0}
      aria-expanded={isExpanded}
      onClick={(event) => {
        if (activatesRow(event)) onToggle();
      }}
      onKeyDown={(event) => {
        if (!activatesRowByKey(event)) return;
        event.preventDefault();
        onToggle();
      }}
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
          "no run recorded"
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
        {row.configured ? "yes" : "no"}
      </TableCell>
      <TableCell className="text-muted-foreground">
        {row.storage
          ? `${row.storage.streams_with_data} of ${row.storage.streams}`
          : "unknown"}
      </TableCell>
      <TableCell className="text-right text-muted-foreground tabular-nums">
        {row.storage
          ? row.storage.physical_rows.toLocaleString("en-US")
          : "unknown"}
      </TableCell>
      <TableCell className="text-right text-muted-foreground tabular-nums">
        {formatBytes(row.storage ? row.storage.bytes_on_disk : null)}
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
                    : `${stream.physical_rows.toLocaleString("en-US")} · ${formatBytes(stream.bytes_on_disk)}`}
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
        <>
          <ul className="grid gap-1 text-sm">
            {data.runs.slice(0, HISTORY_SHOWN).map((event, index) => (
              <li
                key={`${event.event}-${event.started_at}-${index}`}
                className="grid gap-0.5 border-b py-1 last:border-b-0"
              >
                <span className="flex justify-between gap-4">
                  <span className="font-mono text-xs">{event.event}</span>
                  <span className="text-muted-foreground">
                    {describeOutcome(event)}
                  </span>
                </span>
                <span className="text-xs text-muted-foreground">
                  {describeProvenance(event)}
                </span>
              </li>
            ))}
          </ul>
          {/* "of N recorded events" claimed a total the response does not
              carry — it is capped, so N was the cap and not the history. */}
          {data.runs.length > HISTORY_SHOWN && (
            <p className="text-xs text-muted-foreground">
              Showing {HISTORY_SHOWN} of the {data.runs.length} most recent
              recorded events; older ones are not read here.
            </p>
          )}
        </>
      )}
    </section>
  );
}

function describeOutcome(event: RunEvent): string {
  const step = event.step ? ` at ${event.step}` : "";
  // Absence stays silent; a recorded zero is a measurement and gets rendered.
  const took =
    event.duration_ms === null ? "" : ` · ${formatDuration(event.duration_ms)}`;
  return `${event.status}${step} · ${formatAge(event.started_at)}${took}`;
}

/**
 * Who recorded the row, how the sync was started, and what it moved.
 *
 * `origin` is the writer and `trigger` is the cause; the two are never merged,
 * because a swept row says nothing about whether a person started the sync.
 */
function describeProvenance(event: RunEvent): string {
  const parts = [`recorded by ${event.origin}`];
  const trigger = triggerLabel(event.trigger);
  if (trigger) parts.push(trigger);
  if (event.event === "sync.completed") {
    parts.push(formatDelivery(event.records_moved, event.rows_landed));
  }
  return parts.join(" · ");
}
