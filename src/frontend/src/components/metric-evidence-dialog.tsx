import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowUpRight,
  Download,
  FileSpreadsheet,
  FileText,
  Search,
} from "lucide-react";

import {
  downloadMetricDrilldown,
  queryMetricDrilldown,
} from "@/api/metric-drilldown-client";
import { AnalyticsApiError } from "@/api/analytics-client";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import { useAuth } from "@/auth/use-auth";
import type {
  EvidenceDialogState,
  EvidenceDialogTarget,
} from "@/components/metric-evidence-context";
import { MetricEvidencePeople } from "@/components/metric-evidence-people";
import { MetricEvidenceTable } from "@/components/metric-evidence-table";
import {
  SOURCE_DIMENSION,
  withSourceDimension,
  withTypeDimension,
} from "@/lib/metrics/provider-links";
import { useDeclaredMetricDimensions } from "@/queries/metric-definitions";
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
import { Input } from "@/components/ui/input";
import { formatMetricNumber } from "@/lib/format";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import {
  nextSort,
  visibleEvidenceRows,
  type EvidenceSort,
} from "@/lib/metrics/evidence-rows";

/** A records read taken out of a people list, and whose records they are. */
interface DrillStep {
  target: EvidenceDialogTarget;
  personId: string | null;
}

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
  const records = state?.kind === "records" ? state : null;
  const people = state?.kind === "people" ? state.view : null;
  /** The records step taken out of a people list, and whose they are. */
  const [drilled, setDrilled] = useState<DrillStep | null>(null);
  const [peopleSearch, setPeopleSearch] = useState("");
  // A new opening is a new question: the step out of the previous list, and the
  // search that narrowed it, must not survive into it.
  const [openedState, setOpenedState] = useState(state);
  if (openedState !== state) {
    setOpenedState(state);
    setDrilled(null);
    setPeopleSearch("");
  }
  const activeTarget = people
    ? drilled?.target ?? null
    : records?.targets.find(
        (target) => target.selection.metric_key === records.activeMetricKey
      ) ??
      records?.targets[0] ??
      null;
  /** The list itself is on screen only until a row is taken further. */
  const showPeople = people != null && drilled == null;
  const allRecords = people?.allRecords ?? null;
  const headerTitle = drilled
    ? drilled.target.label
    : (people?.title ?? records?.title ?? activeTarget?.label ?? "");
  // INVARIANT: the catalog decides which dimensions may be asked for, so the
  // read waits for it — resolving later would change the selection mid-dialog
  // and refetch every row.
  const declaredDimensions = useDeclaredMetricDimensions();
  const declared = activeTarget
    ? declaredDimensions.byMetricKey?.get(activeTarget.selection.metric_key)
    : null;
  const selection = activeTarget
    ? withTypeDimension(
        withSourceDimension(activeTarget.selection, declared),
        declared
      )
    : null;
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
    enabled:
      sessionScope != null &&
      selection != null &&
      !declaredDimensions.isPending,
    retry: (failureCount, error) =>
      failureCount < 1 &&
      (!(error instanceof AnalyticsApiError) || error.status >= 500),
  });
  const pages = query.data?.pages;
  const rows = useMemo(
    () => pages?.flatMap((page) => page.rows) ?? [],
    [pages]
  );
  const columns = useMemo(() => {
    // `source` rides along purely to back a link (see withSourceDimension) —
    // it is not a column anyone asked to see.
    const columns = (query.data?.pages[0]?.columns ?? []).filter(
      (column) => column.key !== SOURCE_DIMENSION
    );
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
  const canLoadMore = hasNextPage && !pageLimitReached;

  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<EvidenceSort | null>(null);
  const activeMetricKey = selection?.metric_key ?? null;
  const [scopedTo, setScopedTo] = useState(activeMetricKey);
  if (scopedTo !== activeMetricKey) {
    setScopedTo(activeMetricKey);
    setSearch("");
    setSort(null);
  }

  const visiblePeople = useMemo(() => {
    const needle = peopleSearch.trim().toLowerCase();
    const rows = people?.rows ?? [];
    return needle === ""
      ? rows
      : rows.filter((row) => row.name.toLowerCase().includes(needle));
  }, [people, peopleSearch]);

  const narrowed = search.trim() !== "" || sort != null;
  const visibleRows = useMemo(
    () => visibleEvidenceRows({ rows, columns, search, sort }),
    [rows, columns, search, sort]
  );

  // INVARIANT: narrowing must see every page — the table's scroll-triggered
  // paging stalls once a search hides most rows.
  useEffect(() => {
    if (
      narrowed &&
      canLoadMore &&
      !isFetchingNextPage &&
      !query.isFetchNextPageError
    ) {
      void fetchNextPage();
    }
  }, [
    narrowed,
    canLoadMore,
    isFetchingNextPage,
    query.isFetchNextPageError,
    fetchNextPage,
  ]);

  const headingRef = useRef<HTMLSpanElement>(null);
  const focusedStep = useRef(drilled);
  useEffect(() => {
    if (focusedStep.current === drilled) return;
    focusedStep.current = drilled;
    headingRef.current?.focus();
  }, [drilled]);

  useEffect(
    () => () => {
      exportController.current?.abort();
    },
    []
  );

  /** Whatever the export controls were doing belongs to the table being left. */
  function resetExport(): void {
    exportController.current?.abort();
    exportController.current = null;
    setExporting(false);
    setExportFailure(null);
  }

  function closeDialog(): void {
    resetExport();
    onClose();
  }

  /**
   * Move between the people list and one row's records. An export left running
   * over the table being left would resolve onto the next one — a file for a
   * person the reader is no longer looking at, or an error under a body that
   * has no export control at all.
   */
  function goToStep(step: DrillStep | null): void {
    resetExport();
    setDrilled(step);
  }

  async function exportRows(format: "csv" | "xlsx") {
    // INVARIANT: the file holds the columns the screen holds. `source` rides
    // along only so a row can be linked and is hidden from the table, so it is
    // dropped here; `type` is a column the reader can see, so it is kept.
    const exported = activeTarget
      ? withTypeDimension(activeTarget.selection, declared)
      : null;
    if (!exported) return;
    exportController.current?.abort();
    const controller = new AbortController();
    exportController.current = controller;
    setExporting(true);
    setExportFailure(null);
    try {
      await downloadMetricDrilldown(exported, format, controller.signal);
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
      {state && (activeTarget || showPeople) ? (
        <DialogContent className="flex h-[calc(100dvh-2rem)] max-h-[52rem] w-[calc(100vw-2rem)] max-w-none flex-col gap-0 overflow-hidden p-0 sm:h-[calc(100dvh-4rem)] sm:w-[calc(100vw-4rem)] sm:max-w-[90rem] [&_[data-slot=dialog-close]]:top-5">
          <DialogHeader className="shrink-0 border-b p-5 pr-14">
            {people && drilled ? (
              <div className="flex">
                <button
                  type="button"
                  onClick={() => goToStep(null)}
                  aria-label={`Back to ${people.title}`}
                  className="-ms-1 flex cursor-pointer items-center gap-1 rounded-sm px-1 text-xs text-muted-foreground hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                >
                  <ArrowLeft className="size-3.5" aria-hidden />
                  {people.title}
                </button>
              </div>
            ) : null}
            <div className="flex items-center justify-between gap-4">
              {records && records.targets.length > 1 && activeTarget ? (
                <>
                  {/* INVARIANT: the dialog is named for what it shows — a
                      caller that names the whole set wins, otherwise the
                      metric on screen. */}
                  <DialogTitle className="sr-only">
                    {records.title ?? activeTarget.label}
                  </DialogTitle>
                  <Select
                    value={activeTarget.selection.metric_key}
                    onValueChange={(metricKey) => {
                      if (!metricKey) return;
                      resetExport();
                      onMetricChange(metricKey);
                    }}
                  >
                    <SelectTrigger
                      size="sm"
                      aria-label="Metric"
                      className="border-transparent bg-transparent px-0 text-sm font-semibold shadow-none hover:bg-transparent focus-visible:border-transparent focus-visible:ring-0 dark:bg-transparent"
                    >
                      <SelectValue>{activeTarget.label}</SelectValue>
                    </SelectTrigger>
                    <SelectContent align="start">
                      {records.targets.map((target) => (
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
                // Same invariant with one target: a caller that scoped the
                // table to a subset of people named it, and the metric label
                // alone would read as every record of that metric.
                <DialogTitle>
                  {/* Focus lands here on a body swap: the control that was
                      clicked is gone, and the new heading is what says where
                      the reader now is. */}
                  <span ref={headingRef} tabIndex={-1} className="outline-none">
                    {headerTitle}
                  </span>
                </DialogTitle>
              )}
              <div className="flex shrink-0 items-center gap-3">
                {/* Where the records came from, once the dialog is showing one
                    person's — the one exit out of the drill. */}
                {drilled?.personId ? (
                  <Link
                    to="/ic/$person/personal"
                    params={{ person: drilled.personId }}
                    onClick={closeDialog}
                    className="flex items-center gap-1 text-sm text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
                  >
                    Person page
                    <ArrowUpRight className="size-3.5" aria-hidden />
                  </Link>
                ) : null}
                {/* Export serves the record table; a people list is a read of
                    values this session already had. */}
                {showPeople ? null : (
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
                )}
              </div>
            </div>
            {exportFailure ? (
              <p role="alert" className="text-sm text-destructive">
                {exportFailure}
              </p>
            ) : null}
            {showPeople && people ? (
              <div className="flex flex-wrap items-center gap-3">
                <div className="relative min-w-0 flex-1 sm:max-w-xs">
                  <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
                  <Input
                    type="search"
                    value={peopleSearch}
                    onChange={(event) => setPeopleSearch(event.target.value)}
                    placeholder="Search people"
                    aria-label="Search people"
                    className="h-8 ps-8"
                  />
                </div>
                <p
                  aria-live="polite"
                  className="text-sm text-muted-foreground tabular-nums"
                >
                  {peopleCount(visiblePeople.length, people.rows.length)}
                </p>
                {allRecords ? (
                  <Button
                    variant="outline"
                    size="sm"
                    className="ms-auto"
                    onClick={() =>
                      goToStep({ target: allRecords, personId: null })
                    }
                  >
                    All records
                  </Button>
                ) : null}
              </div>
            ) : query.isPending || (query.isError && !query.data) ? null : (
              <div className="flex flex-wrap items-center gap-3">
                <div className="relative min-w-0 flex-1 sm:max-w-xs">
                  <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
                  <Input
                    type="search"
                    value={search}
                    onChange={(event) => setSearch(event.target.value)}
                    placeholder="Search records"
                    aria-label="Search records"
                    className="h-8 ps-8"
                  />
                </div>
                <p
                  aria-live="polite"
                  className="text-sm text-muted-foreground tabular-nums"
                >
                  {recordCount({
                    visible: visibleRows.length,
                    loaded: rows.length,
                    filtered: search.trim() !== "",
                    partial:
                      narrowed && (canLoadMore || query.isFetchNextPageError),
                  })}
                </p>
              </div>
            )}
          </DialogHeader>
          {showPeople ? (
            visiblePeople.length === 0 ? (
              <div className="flex flex-1 flex-col items-center justify-center gap-3">
                <p className="text-sm text-muted-foreground">
                  No people match this search
                </p>
                <Button variant="outline" onClick={() => setPeopleSearch("")}>
                  Clear search
                </Button>
              </div>
            ) : (
              <MetricEvidencePeople
                rows={visiblePeople}
                valueLabel={people.valueLabel}
                onDrill={(row) =>
                  row.target &&
                  goToStep({ target: row.target, personId: row.personId })
                }
              />
            )
          ) : query.isPending ? (
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
          ) : visibleRows.length === 0 ? (
            <div className="flex flex-1 flex-col items-center justify-center gap-3">
              {query.isFetchNextPageError ? (
                // SAFETY: only loaded pages were searched — "no match" would
                // claim something about records nobody has seen.
                <>
                  <p role="alert" className="text-sm text-muted-foreground">
                    Nothing matched the records loaded so far, and the rest
                    could not be loaded
                  </p>
                  <div className="flex gap-2">
                    <Button
                      variant="outline"
                      onClick={() => void fetchNextPage()}
                    >
                      Retry
                    </Button>
                    <Button variant="ghost" onClick={() => setSearch("")}>
                      Clear search
                    </Button>
                  </div>
                </>
              ) : canLoadMore ? (
                <p className="text-sm text-muted-foreground">
                  Nothing matched yet — still loading the rest
                </p>
              ) : (
                <>
                  <p className="text-sm text-muted-foreground">
                    No records match this search
                  </p>
                  <Button variant="outline" onClick={() => setSearch("")}>
                    Clear search
                  </Button>
                </>
              )}
            </div>
          ) : (
            <MetricEvidenceTable
              // INVARIANT: remounts per metric — expansion state is the
              // table's and must not carry across.
              key={activeMetricKey}
              metricKey={activeMetricKey}
              rows={visibleRows}
              columns={columns}
              sort={sort}
              onSortChange={(key) =>
                setSort((current) => nextSort(current, key))
              }
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

function recordCount({
  visible,
  loaded,
  filtered,
  partial,
}: {
  visible: number;
  loaded: number;
  filtered: boolean;
  partial: boolean;
}): string {
  const noun = visible === 1 && !filtered ? "record" : "records";
  const count = filtered
    ? `${formatMetricNumber(visible, "integer")} of ${formatMetricNumber(loaded, "integer")}`
    : formatMetricNumber(visible, "integer");
  return `${count} ${noun}${partial ? " so far" : ""}`;
}

/** "5 people", or "2 of 5 people" once a search has narrowed the list. */
function peopleCount(visible: number, total: number): string {
  const noun = visible === 1 && visible === total ? "person" : "people";
  const count =
    visible === total
      ? formatMetricNumber(visible, "integer")
      : `${formatMetricNumber(visible, "integer")} of ${formatMetricNumber(total, "integer")}`;
  return `${count} ${noun}`;
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
