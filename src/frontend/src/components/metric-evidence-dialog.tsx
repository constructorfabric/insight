import { useInfiniteQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";
import { Download, FileSpreadsheet, FileText } from "lucide-react";

import {
  downloadMetricDrilldown,
  queryMetricDrilldown,
} from "@/api/metric-drilldown-client";
import { AnalyticsApiError } from "@/api/analytics-client";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import { useAuth } from "@/auth/use-auth";
import type { EvidenceDialogState } from "@/components/metric-evidence-context";
import { MetricEvidenceTable } from "@/components/metric-evidence-table";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";

export function MetricEvidenceDialog({
  state,
  onMetricChange,
  onClose,
}: {
  state: EvidenceDialogState | null;
  onMetricChange: (metricKey: string) => void;
  onClose: () => void;
}) {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  const exportController = useRef<AbortController | null>(null);
  const [exporting, setExporting] = useState(false);
  const [exportFailure, setExportFailure] = useState<string | null>(null);
  const activeTarget =
    state?.targets.find(
      (target) => target.selection.metric_key === state.activeMetricKey
    ) ??
    state?.targets[0] ??
    null;
  const selection = activeTarget?.selection ?? null;
  const query = useInfiniteQuery({
    queryKey: ["metric-drilldown", sessionScope, selection],
    queryFn: ({ pageParam, signal }) => {
      if (!selection) throw new Error("Metric evidence selection is missing");
      return queryMetricDrilldown(
        { ...selection, cursor: pageParam, limit: 100 },
        signal
      );
    },
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: sessionScope != null && selection != null,
    retry: (failureCount, error) =>
      failureCount < 1 &&
      (!(error instanceof AnalyticsApiError) || error.status >= 500),
  });
  const rows = query.data?.pages.flatMap((page) => page.rows) ?? [];
  const columns = useMemo(() => {
    const columns = query.data?.pages[0]?.columns ?? [];
    const order = new Map([
      ["ref", 0],
      ["title", 1],
      ["repository", 2],
      ["author", 3],
      ["date", 100],
      ["value", 101],
      ["numerator", 101],
      ["denominator", 102],
    ]);
    return [...columns].sort(
      (left, right) =>
        (order.get(left.key) ?? 50) - (order.get(right.key) ?? 50) ||
        left.label.localeCompare(right.label)
    );
  }, [query.data?.pages]);
  const { fetchNextPage, hasNextPage, isFetchingNextPage } = query;
  const pageLimitReached = (query.data?.pages.length ?? 0) >= 50 && hasNextPage;

  useEffect(
    () => () => {
      exportController.current?.abort();
    },
    []
  );

  function closeDialog(): void {
    exportController.current?.abort();
    exportController.current = null;
    setExporting(false);
    setExportFailure(null);
    onClose();
  }

  async function exportRows(format: "csv" | "xlsx") {
    if (!selection) return;
    exportController.current?.abort();
    const controller = new AbortController();
    exportController.current = controller;
    setExporting(true);
    setExportFailure(null);
    try {
      await downloadMetricDrilldown(selection, format, controller.signal);
    } catch (error) {
      if (!controller.signal.aborted) {
        setExportFailure(
          errorMessage(error, "Unable to export metric evidence")
        );
      }
    } finally {
      if (exportController.current === controller) {
        exportController.current = null;
        setExporting(false);
      }
    }
  }

  return (
    <Dialog
      open={state != null}
      onOpenChange={(open) => !open && closeDialog()}
    >
      {state && activeTarget ? (
        <DialogContent className="flex h-[calc(100dvh-2rem)] max-h-[52rem] w-[calc(100vw-2rem)] max-w-none flex-col gap-0 overflow-hidden p-0 sm:h-[calc(100dvh-4rem)] sm:w-[calc(100vw-4rem)] sm:max-w-[90rem] [&_[data-slot=dialog-close]]:top-5">
          <DialogHeader className="shrink-0 border-b p-5 pr-14">
            <div className="flex items-center justify-between gap-4">
              {state.targets.length > 1 ? (
                <>
                  <DialogTitle className="sr-only">
                    {state.title ?? "Metric evidence"}
                  </DialogTitle>
                  <Select
                    value={activeTarget.selection.metric_key}
                    onValueChange={(metricKey) => {
                      if (!metricKey) return;
                      exportController.current?.abort();
                      setExportFailure(null);
                      onMetricChange(metricKey);
                    }}
                  >
                    <SelectTrigger
                      size="sm"
                      aria-label="Metric"
                      className="border-transparent bg-transparent px-0 text-base font-semibold shadow-none hover:bg-transparent focus-visible:border-transparent focus-visible:ring-0 dark:bg-transparent"
                    >
                      <SelectValue>{activeTarget.label}</SelectValue>
                    </SelectTrigger>
                    <SelectContent align="start">
                      {state.targets.map((target) => (
                        <SelectItem
                          key={target.selection.metric_key}
                          value={target.selection.metric_key}
                        >
                          {target.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </>
              ) : (
                <DialogTitle>{activeTarget.label}</DialogTitle>
              )}
              <DropdownMenu>
                <DropdownMenuTrigger
                  disabled={
                    exporting ||
                    query.isPending ||
                    (query.isError && !query.data)
                  }
                  render={
                    <Button variant="outline" size="sm">
                      {exporting ? <Spinner /> : <Download />}
                      Export
                    </Button>
                  }
                />
                <DropdownMenuContent align="end">
                  <DropdownMenuItem onClick={() => void exportRows("csv")}>
                    <FileText />
                    CSV
                  </DropdownMenuItem>
                  <DropdownMenuItem onClick={() => void exportRows("xlsx")}>
                    <FileSpreadsheet />
                    Excel
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
            {exportFailure ? (
              <p role="alert" className="text-sm text-destructive">
                {exportFailure}
              </p>
            ) : null}
          </DialogHeader>
          {query.isPending ? (
            <div className="flex flex-1 items-center justify-center">
              <Spinner className="size-10" />
            </div>
          ) : query.isError && !query.data ? (
            <div className="flex flex-1 flex-col items-center justify-center gap-3">
              <p role="alert" className="text-sm text-muted-foreground">
                {errorMessage(query.error, "Unable to load metric evidence")}
              </p>
              <Button variant="outline" onClick={() => void query.refetch()}>
                Retry
              </Button>
            </div>
          ) : rows.length === 0 ? (
            <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
              No supporting data for this selection
            </div>
          ) : (
            <MetricEvidenceTable
              rows={rows}
              columns={columns}
              fetchNextPage={fetchNextPage}
              hasNextPage={hasNextPage && !pageLimitReached}
              isFetchingNextPage={isFetchingNextPage}
              nextPageError={query.isFetchNextPageError}
              pageLimitReached={pageLimitReached}
            />
          )}
        </DialogContent>
      ) : null}
    </Dialog>
  );
}

function errorMessage(error: unknown, fallback: string): string {
  if (
    !(error instanceof AnalyticsApiError) ||
    !error.body ||
    typeof error.body !== "object"
  ) {
    return fallback;
  }
  const problem = error.body as { detail?: unknown; trace_id?: unknown };
  const detail = typeof problem.detail === "string" ? problem.detail : fallback;
  return typeof problem.trace_id === "string"
    ? `${detail} Trace: ${problem.trace_id}`
    : detail;
}
