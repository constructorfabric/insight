import { useMemo, useRef, useState } from "react";
import { Download, FileSpreadsheet, FileText, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Checkbox } from "@/components/ui/checkbox";
import { Spinner } from "@/components/ui/spinner";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { usePortalPeriod } from "@/hooks/use-portal-period";
import { downloadMatrixCsv, downloadMatrixXlsx } from "@/lib/export/matrix";
import { normalizePersonId } from "@/lib/metrics/entity";
import { useOrgScope } from "@/lib/portal/use-org-scope";
import { unavailableReason } from "@/lib/reports/availability";
import { byFamily } from "@/lib/reports/families";
import { buildReportTable, type ReportTable } from "@/lib/reports/report-table";
import { collectReportPeople } from "@/lib/reports/roster-columns";
import {
  bucketsInRange,
  needsRollup,
  requestBucket,
  type ReportGranularity,
} from "@/lib/reports/rollup";
import { runReport } from "@/lib/reports/run-report";
import { cn } from "@/lib/utils";
import { TEXT_EYEBROW, TEXT_LABEL, TEXT_TITLE } from "@/lib/type-scale";
import { useMetricComputations } from "@/queries/report-catalogue";
import { useMetricDefinitionsResponse } from "@/queries/metric-definitions";

const GRANULARITIES: ReadonlyArray<{ value: ReportGranularity; label: string }> = [
  { value: "day", label: "Daily" },
  { value: "week", label: "Weekly" },
  { value: "month", label: "Monthly" },
  { value: "quarter", label: "Quarterly" },
  { value: "year", label: "Yearly" },
];

/** Enough to see the shape of the file without rendering the whole of it. */
const PREVIEW_ROWS = 20;

export function ReportBuilderView() {
  const { dateRange } = usePortalPeriod();
  const scope = useOrgScope();
  const definitions = useMetricDefinitionsResponse();
  const computations = useMetricComputations();

  const [selected, setSelected] = useState<string[]>([]);
  const [granularity, setGranularity] = useState<ReportGranularity>("month");
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(
    null,
  );
  const [table, setTable] = useState<ReportTable | null>(null);
  const [builtFor, setBuiltFor] = useState<string | null>(null);
  const [builtAt, setBuiltAt] = useState<string | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [previewOpen, setPreviewOpen] = useState(false);
  const abort = useRef<AbortController | null>(null);

  // Everything enabled is listed; what this installation cannot serve is shown
  // unavailable rather than omitted. The results endpoint rejects an
  // unreachable key outright, so it must not be selectable — but dropping it
  // silently takes whole families off the screen and leaves the reader hunting
  // for a section that is simply not measured here.
  const catalogue = useMemo(
    () => (definitions.data?.metrics ?? []).filter((metric) => metric.is_enabled),
    [definitions.data],
  );
  // The additivity restriction belongs to the rollup, not to the report. At a
  // bucket the server has, it computed each value itself — a ratio is that
  // bucket's own ratio — so the whole catalogue is fair game. Only a quarter
  // and a year are months added together, and only there does the question
  // "may this be added" arise.
  // Nothing is dropped from the list. A metric that cannot go in this report
  // is shown with the reason, because a metric that simply vanishes leaves the
  // reader unable to tell "not measured here" from "I misremembered the name".
  const offered = useMemo(
    () =>
      catalogue.map((metric) => ({
        ...metric,
        reason: unavailableReason(
          metric,
          granularity,
          computations.data ?? new Map(),
        ),
      })),
    [catalogue, computations.data, granularity],
  );

  const people = useMemo(() => {
    const attrs = collectReportPeople(scope.pivot);
    return (scope.roster ?? []).flatMap((entry) => {
      const person = attrs.get(normalizePersonId(entry.person_id));
      return person ? [person] : [];
    });
  }, [scope.pivot, scope.roster]);

  const selectedMetrics = useMemo(
    () =>
      selected.flatMap((key) => {
        const metric = offered.find((m) => m.metric_key === key && !m.reason);
        return metric ? [{ metric_key: key, label: metric.label }] : [];
      }),
    [selected, offered],
  );

  // What the table on hand was built from. A report is not stored anywhere —
  // it lives here until something it depends on changes, and then it is wrong
  // rather than merely old: build it monthly, switch to yearly, and the button
  // would still offer the monthly file under the new heading.
  const recipe = [
    granularity,
    dateRange.from,
    dateRange.to,
    // The people themselves, not how many: two different rosters of equal size
    // would otherwise look like the same report, and a table built for one
    // scope would stay downloadable under another.
    ...people.map((person) => person.entityId),
    "|",
    ...selectedMetrics.map((metric) => metric.metric_key),
  ].join(" ");
  if (table && builtFor !== recipe) {
    setTable(null);
    setBuiltFor(null);
    setBuiltAt(null);
    setPreviewOpen(false);
  }

  const running = progress != null;
  const canBuild = selectedMetrics.length > 0 && people.length > 0 && !running;

  async function build(): Promise<void> {
    const controller = new AbortController();
    abort.current = controller;
    setFailure(null);
    setTable(null);
    try {
      const results = await runReport({
        metricKeys: selectedMetrics.map((m) => m.metric_key),
        entityIds: people.map((person) => person.entityId),
        range: dateRange,
        granularity,
        bucketCount: bucketsInRange(
          dateRange.from,
          dateRange.to,
          requestBucket(granularity),
        ).length,
        onProgress: (done, total) => setProgress({ done, total }),
        signal: controller.signal,
      });
      setBuiltFor(recipe);
      setBuiltAt(
        new Date().toLocaleString(undefined, {
          dateStyle: "medium",
          timeStyle: "short",
        }),
      );
      setPreviewOpen(true);
      setTable(
        buildReportTable({
          people,
          metrics: selectedMetrics,
          results,
          range: dateRange,
          granularity,
        }),
      );
    } catch (error) {
      // A run that stopped early produces nothing: a file missing a few
      // batches reads as a complete one once it is open.
      setTable(null);
      setFailure(
        controller.signal.aborted
          ? "Cancelled — nothing was downloaded."
          : `Could not build the report: ${(error as Error).message}`,
      );
    } finally {
      setProgress(null);
      abort.current = null;
    }
  }

  // The granularity is in the name too: two files for the same period that
  // differ only in their buckets would otherwise overwrite each other in a
  // downloads folder.
  const filename = `insight-report_${granularity}_${dateRange.from}_${dateRange.to}`;

  if (computations.isError || definitions.isError) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon variant="card" state="error" label="Unable to load the metric catalogue" />
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4 p-4 md:p-6">
      <div className="flex flex-col gap-1">
        <h1 className={TEXT_TITLE}>Report builder</h1>
        <p className="text-sm text-muted-foreground">
          {scope.label ? `${scope.label} · ` : ""}
          {people.length} {people.length === 1 ? "person" : "people"} · one row
          per person per period
        </p>
      </div>

      <Card>
        <CardContent className="flex flex-col gap-4 p-4">
          <div className="flex flex-wrap items-center gap-3">
            <span className={TEXT_LABEL}>Granularity</span>
            <ToggleGroup
              value={[granularity]}
              onValueChange={(value) => {
                const next = Array.isArray(value) ? value[0] : value;
                if (next) setGranularity(next as ReportGranularity);
              }}
              variant="outline"
              size="sm"
            >
              {GRANULARITIES.map((option) => (
                <ToggleGroupItem key={option.value} value={option.value}>
                  {option.label}
                </ToggleGroupItem>
              ))}
            </ToggleGroup>
            <span className="text-xs text-muted-foreground">
              The period comes from the bar above.
            </span>
          </div>

          <div className="flex flex-col gap-2">
            <span className={TEXT_LABEL}>
              {needsRollup(granularity)
                ? "Metrics that can be totalled — a quarter is its months added up"
                : "Metrics"}
            </span>
            {needsRollup(granularity) && computations.isPending ? (
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <Spinner className="size-4" /> Reading the catalogue…
              </div>
            ) : (
              <TooltipProvider delay={300}>
              <div className="flex flex-col gap-4">
                {byFamily(offered).map((group) => {
                  const keys = group.metrics
                    .filter((m) => !m.reason)
                    .map((m) => m.metric_key);
                  const allPicked =
                    keys.length > 0 &&
                    keys.every((key) => selected.includes(key));
                  return (
                    <div key={group.family} className="flex flex-col gap-1">
                      <div className="flex items-center gap-2">
                        <span className={TEXT_EYEBROW}>{group.name}</span>
                        {keys.length > 0 ? (
                          <Button
                            type="button"
                            variant="ghost"
                            size="xs"
                            onClick={() =>
                              setSelected((current) =>
                                allPicked
                                  ? current.filter((key) => !keys.includes(key))
                                  : [...new Set([...current, ...keys])],
                              )
                            }
                          >
                            {allPicked ? "None" : "All"}
                          </Button>
                        ) : (
                          <span className="text-xs text-muted-foreground">
                            nothing measured here yet
                          </span>
                        )}
                      </div>
                      <div className="grid grid-cols-1 gap-1 sm:grid-cols-2 lg:grid-cols-3">
                        {group.metrics.map((metric) => {
                          const checked = selected.includes(metric.metric_key);
                          return (
                            <Tooltip key={metric.metric_key}>
                              <TooltipTrigger
                                render={
                            <label
                              htmlFor={`report-${metric.metric_key}`}
                              // A native title as well as the tooltip: the
                              // reason a metric cannot be picked is the one
                              // thing here that must reach the reader, and a
                              // tooltip trigger on a label wrapping a disabled
                              // control is not something to bet it on.
                              title={metric.reason ?? undefined}
                              className={cn(
                                "flex items-center gap-2 rounded-sm px-1 py-1 text-start text-sm",
                                metric.reason
                                  ? "text-muted-foreground"
                                  : "cursor-pointer hover:bg-muted",
                              )}
                            >
                              <Checkbox
                                id={`report-${metric.metric_key}`}
                                checked={checked}
                                disabled={Boolean(metric.reason)}
                                onCheckedChange={() =>
                                  setSelected((current) =>
                                    checked
                                      ? current.filter(
                                          (key) => key !== metric.metric_key,
                                        )
                                      : [...current, metric.metric_key],
                                  )
                                }
                              />
                              {metric.label}
                            </label>
                                }
                              />
                              <TooltipContent
                                side="top"
                                className="max-w-xs text-xs leading-relaxed"
                              >
                                {metric.reason ? (
                                  <>
                                    <span className="font-medium">
                                      Not available here.
                                    </span>{" "}
                                    {metric.reason}
                                  </>
                                ) : (
                                  metric.description ??
                                  metric.explanation ??
                                  metric.label
                                )}
                              </TooltipContent>
                            </Tooltip>
                          );
                        })}
                      </div>
                    </div>
                  );
                })}
              </div>
              </TooltipProvider>
            )}
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <Button type="button" disabled={!canBuild} onClick={() => void build()}>
              Build report
            </Button>
            {running ? (
              <>
                <span className="text-sm text-muted-foreground">
                  {progress.done} of {progress.total} batches
                </span>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => abort.current?.abort()}
                >
                  <X className="size-4" />
                  Cancel
                </Button>
              </>
            ) : null}
            {selected.length > 0 && !running ? (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => setSelected([])}
              >
                Clear selection
              </Button>
            ) : null}
          </div>

          {failure ? (
            <p className="text-sm text-destructive">{failure}</p>
          ) : null}
        </CardContent>
      </Card>

      {table ? (
        <Button
          type="button"
          variant="outline"
          className="self-start"
          onClick={() => setPreviewOpen(true)}
        >
          <Download className="size-4" />
          {table.rows.length} rows ·{" "}
          {GRANULARITIES.find((g) => g.value === granularity)?.label} ·{" "}
          {dateRange.from} to {dateRange.to}
          {builtAt ? ` · built ${builtAt}` : ""}
        </Button>
            ) : null}

      <Dialog open={previewOpen && table != null} onOpenChange={setPreviewOpen}>
        <DialogContent className="flex max-h-[85vh] flex-col gap-3 sm:max-w-[min(96vw,1200px)]">
          <DialogHeader>
            <DialogTitle>
              {table?.rows.length ?? 0} rows · showing the first{" "}
              {Math.min(PREVIEW_ROWS, table?.rows.length ?? 0)}
            </DialogTitle>
            <p className="text-xs text-muted-foreground">
              {GRANULARITIES.find((g) => g.value === granularity)?.label} ·{" "}
              {dateRange.from} to {dateRange.to} · {people.length} people
              {builtAt ? ` · built ${builtAt}` : ""}
            </p>
          </DialogHeader>
          {table ? (
            <>
              <Table className="text-xs" containerClassName="overflow-auto">
                <TableHeader>
                  <TableRow>
                    {table.columns.map((column) => (
                      <TableHead key={column} className="whitespace-nowrap">
                        {column}
                      </TableHead>
                    ))}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {table.rows.slice(0, PREVIEW_ROWS).map((row, index) => (
                    <TableRow key={index}>
                      {row.map((cell, cellIndex) => (
                        <TableCell
                          key={cellIndex}
                          className="whitespace-nowrap tabular-nums"
                        >
                          {cell ?? "—"}
                        </TableCell>
                      ))}
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
              <div className="flex items-center justify-end gap-2">
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => downloadMatrixCsv(`${filename}.csv`, table)}
                >
                  <FileText className="size-4" />
                  CSV
                </Button>
                <Button
                  type="button"
                  size="sm"
                  onClick={() =>
                    void downloadMatrixXlsx(`${filename}.xlsx`, "Report", table)
                  }
                >
                  <FileSpreadsheet className="size-4" />
                  Excel
                </Button>
              </div>
            </>
          ) : null}
        </DialogContent>
      </Dialog>
    </div>
  );
}
