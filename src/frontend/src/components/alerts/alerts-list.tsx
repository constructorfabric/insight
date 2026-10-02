import { Link } from "@tanstack/react-router";
import { useInfiniteQuery } from "@tanstack/react-query";
import { useState } from "react";

import type { AlertSummary } from "@/api/alerts-client";
import { AlertSwitch } from "@/components/alerts/alert-switch";
import {
  DefinitionCount,
  MoreDefinitions,
} from "@/components/custom/definition-paging";
import { DefinitionSearch } from "@/components/custom/definition-search";
import { refusal } from "@/components/custom/refusal";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import { TEXT_BODY, TEXT_NAME, TEXT_TITLE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import { alertPagesQuery } from "@/queries/alerts";

/** Long enough that a word is typed before the service hears about it. */
const SEARCH_DEBOUNCE_MS = 400;

export function AlertsList() {
  const [needle, setNeedle] = useState("");
  const searching = useDebouncedValue(needle, SEARCH_DEBOUNCE_MS).trim();
  const pages = useInfiniteQuery(alertPagesQuery(searching));
  const alerts = pages.data?.pages.flatMap((page) => page.alerts);

  return (
    <div className="flex flex-col gap-4 p-4 md:p-6">
      <header className="flex flex-wrap items-start gap-3">
        <div className="min-w-0 grow">
          <h1 className={TEXT_TITLE}>Alerts</h1>
          <p className={cn(TEXT_BODY, "text-muted-foreground")}>
            Each alert checks one number a metric produces and sends a
            notification when it crosses a threshold.
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          nativeButton={false}
          render={<Link to="/portal/custom/alerts/new" />}
        >
          New alert
        </Button>
      </header>

      <div className="flex flex-wrap items-center gap-3">
        <DefinitionSearch
          label="Search alerts"
          value={needle}
          onChange={setNeedle}
        />
        <DefinitionCount
          total={pages.data?.pages.at(-1)?.total}
          noun="alerts"
          searching={searching !== ""}
        />
      </div>

      {pages.isPending ? (
        <CenteredSpinner className="min-h-40" />
      ) : pages.isError ? (
        <ComingSoon
          variant="card"
          state="error"
          label={refusal(pages.error, "Couldn't load the alerts.")}
          onRetry={() => void pages.refetch()}
        />
      ) : !alerts || alerts.length === 0 ? (
        <ComingSoon
          variant="card"
          state="empty"
          label={
            searching
              ? "No alert matches that search."
              : "No alerts yet. Create one to be notified when a metric crosses a threshold."
          }
        />
      ) : (
        <>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Metric</TableHead>
                <TableHead className="w-28 text-end">Checks</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {alerts.map((alert) => (
                <AlertRow key={alert.id} alert={alert} />
              ))}
            </TableBody>
          </Table>
          <MoreDefinitions
            hasMore={pages.hasNextPage}
            isFetchingMore={pages.isFetchingNextPage}
            onMore={() => void pages.fetchNextPage()}
          />
        </>
      )}
    </div>
  );
}

function AlertRow({ alert }: { alert: AlertSummary }) {
  return (
    <TableRow>
      <TableCell className="w-1/2 max-w-0">
        <Link
          to="/portal/custom/alerts/$id"
          params={{ id: alert.id }}
          className={cn(
            TEXT_NAME,
            "block truncate underline-offset-4 hover:underline"
          )}
        >
          {alert.name}
        </Link>
      </TableCell>
      <TableCell className="max-w-0">
        <Link
          to="/portal/custom/metrics/$name"
          params={{ name: alert.metric }}
          className={cn(
            TEXT_BODY,
            "block truncate font-mono text-muted-foreground underline-offset-4 hover:underline"
          )}
        >
          {alert.metric}
        </Link>
      </TableCell>
      <TableCell className="text-end">
        <AlertSwitch id={alert.id} name={alert.name} enabled={alert.enabled} />
      </TableCell>
    </TableRow>
  );
}
