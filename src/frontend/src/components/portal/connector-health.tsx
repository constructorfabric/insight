import { useState } from "react";

import type { SyncFact } from "@/api/connector-health-client";
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
import {
  UNMEASURED,
  describeConnector,
  describeRecording,
  describeSync,
  formatDuration,
  formatRecords,
  formatStarted,
  type ConnectorTone,
} from "@/lib/portal/connector-health";
import { useConnectorHealth, useConnectorSyncs } from "@/queries/connector-health";
import { activatesRow, activatesRowByKey } from "@/lib/identities/row-activation";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/** Tone carries emphasis; the word beside it carries the meaning. */
const TONE_STYLE: Record<ConnectorTone, string> = {
  failing: "bg-destructive/15 text-destructive",
  unknown: "bg-warning/15 text-warning",
  active: "bg-primary/15 text-primary",
  ok: "bg-success/15 text-success",
  idle: "bg-muted text-muted-foreground",
};

const COLUMNS = 5;

export function ConnectorHealthPane() {
  const { data, isPending, isError, refetch } = useConnectorHealth();
  const [expanded, setExpanded] = useState<string | null>(null);

  if (isPending) return <CenteredSpinner className="min-h-[60vh]" />;
  if (isError || data === undefined) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon variant="card" state="error" onRetry={() => refetch()} />
      </div>
    );
  }

  const recording = describeRecording(data);

  return (
    <div className="flex flex-col gap-3 p-4 md:p-6">
      <div>
        <h1 className="text-lg font-semibold tracking-tight">
          Connector health
        </h1>
        <div
          role="status"
          aria-live="polite"
          className={cn(
            "text-sm",
            recording.state === "stopped" || recording.state === "unreadable"
              ? "text-destructive"
              : "text-muted-foreground",
          )}
        >
          <p>{recording.label}</p>
          {/* Inside the region, not beside it: the warning's consequence is the
              half a reader most needs announced. */}
          {recording.detail !== "" && <p>{recording.detail}</p>}
        </div>
      </div>

      {data.connectors.length === 0 ? (
        <div className="rounded-lg border p-6 text-sm text-muted-foreground">
          {recording.state === "never_read"
            ? "No connector has been recorded yet. This page reports what has been read from the data mover, and nothing has been read."
            : "The data mover was read and reported no connector. Nothing here says a connector is missing — only that none was recorded."}
        </div>
      ) : (
        <div className="overflow-x-auto rounded-lg border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Connector</TableHead>
                <TableHead>State</TableHead>
                <TableHead>Last sync started</TableHead>
                <TableHead className="text-right">Duration</TableHead>
                <TableHead className="text-right">Records reported</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.connectors.map((row) => {
                const state = describeConnector(row);
                // Keyed on the connector, not on the row's position. The page
                // polls, so a position can come to mean a different connector
                // between renders and the wrong row would open. The name is
                // safe to key on: the read groups by connector, so the response
                // cannot carry two rows sharing one.
                const open = expanded === row.connector;
                const panelId = `connector-syncs-${row.connector}`;
                const toggle = () => setExpanded(open ? null : row.connector);
                return [
                  // The whole row opens the connector, by the same rule every
                  // console listing follows — `activatesRow` is shared with
                  // them, so a press on a control and the end of a text
                  // selection are told apart here exactly as they are there.
                  //
                  // The gesture goes on the row and the role does not: a `<tr>`
                  // keeps `role="row"`, which supports `aria-expanded`, so the
                  // disclosure is announced without taking the row out of its
                  // table. An inner button would be a second focus stop for the
                  // one thing the row already does.
                  <TableRow
                    key={row.connector}
                    data-state-name={state.state}
                    data-state={open ? "selected" : undefined}
                    tabIndex={0}
                    aria-expanded={open}
                    aria-controls={panelId}
                    onClick={(event) => {
                      if (activatesRow(event)) toggle();
                    }}
                    onKeyDown={(event) => {
                      if (!activatesRowByKey(event)) return;
                      event.preventDefault();
                      toggle();
                    }}
                    className="cursor-pointer select-text"
                  >
                    <TableCell className="font-medium">{row.connector}</TableCell>
                    <TableCell>
                      <Badge
                        variant="secondary"
                        className={cn("font-medium", TONE_STYLE[state.tone])}
                      >
                        {state.label}
                      </Badge>
                    </TableCell>
                    <TableCell className="tabular-nums text-muted-foreground">
                      {formatStarted(row.last_sync?.started_at ?? null)}
                    </TableCell>
                    <TableCell className="text-right tabular-nums text-muted-foreground">
                      {formatDuration(row.last_sync?.duration_ms ?? null)}
                    </TableCell>
                    <TableCell className="text-right tabular-nums text-muted-foreground">
                      {formatRecords(row.last_sync?.records_reported ?? null)}
                    </TableCell>
                  </TableRow>,
                  open ? (
                    <TableRow key={`syncs-${row.connector}`}>
                      <TableCell colSpan={COLUMNS} id={panelId} className="bg-muted/40">
                        <RecentSyncs connector={row.connector} />
                      </TableCell>
                    </TableRow>
                  ) : null,
                ];
              })}
            </TableBody>
          </Table>
        </div>
      )}
    </div>
  );
}

function RecentSyncs({ connector }: { connector: string }) {
  const { data, isPending, isError, refetch } = useConnectorSyncs(connector);

  if (isPending) return <CenteredSpinner className="min-h-24" />;
  if (isError || data === undefined) {
    return (
      <div className="max-w-md">
        <ComingSoon variant="card" state="error" onRetry={() => refetch()} />
      </div>
    );
  }
  if (data.syncs.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        No sync has been recorded for this connector.
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      <p className={TEXT_LABEL}>
        Recent syncs — the most recent {data.window}, not the full history
      </p>
      <ul className="flex flex-col gap-1">
        {data.syncs.map((sync) => (
          <SyncLine key={sync.job_id} sync={sync} />
        ))}
      </ul>
    </div>
  );
}

function SyncLine({ sync }: { sync: SyncFact }) {
  const state = describeSync(sync);
  return (
    <li className="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm">
      <Badge className={cn("font-medium", TONE_STYLE[state.tone])}>
        {state.label}
      </Badge>
      <span className="tabular-nums">{formatStarted(sync.started_at)}</span>
      <span className="tabular-nums text-muted-foreground">
        {formatDuration(sync.duration_ms)}
      </span>
      <span className="tabular-nums text-muted-foreground">
        {formatRecords(sync.records_reported)} records
      </span>
      {sync.records_reported == null && (
        <span className="sr-only">
          {UNMEASURED} means the mover reported no count, not a count of zero
        </span>
      )}
    </li>
  );
}
