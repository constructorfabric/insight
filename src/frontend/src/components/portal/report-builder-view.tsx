import { useEffect, useMemo, useRef, useState } from "react";
import { Download, X } from "lucide-react";

import type {
  ReportGranularity,
  ReportPreviewResponse,
  ReportRecipe,
} from "@/api/reports-client";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Spinner } from "@/components/ui/spinner";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { ComingSoon } from "@/components/widgets/coming-soon";
import {
  ReportMetricPicker,
  type OfferedReportMetric,
} from "@/components/portal/report-builder-metrics";
import { ReportPreviewDialog } from "@/components/portal/report-builder-preview";
import { usePortalPeriod } from "@/hooks/use-portal-period";
import { metricVisible } from "@/lib/portal/nav-policy";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { useOrgScope } from "@/lib/portal/use-org-scope";
import { byFamily } from "@/lib/reports/families";
import { TEXT_LABEL, TEXT_TITLE } from "@/lib/type-scale";
import { useMetricDefinitionsResponse } from "@/queries/metric-definitions";
import { useReportExport, useReportPreview } from "@/queries/reports";
import { recordUsageEvent } from "@/telemetry";

const GRANULARITIES: ReadonlyArray<{
  value: ReportGranularity;
  label: string;
}> = [
  { value: "day", label: "Daily" },
  { value: "week", label: "Weekly" },
  { value: "month", label: "Monthly" },
  { value: "quarter", label: "Quarterly" },
  { value: "year", label: "Yearly" },
];

type ReportSubjectKind = "people" | "tenant";

interface PreviewState {
  recipe: string;
  response: ReportPreviewResponse;
}

export function ReportBuilderView() {
  const { dateRange } = usePortalPeriod();
  const scope = useOrgScope();
  const definitions = useMetricDefinitionsResponse();
  const previewRequest = useReportPreview();
  const exportRequest = useReportExport();
  const showPlanned = usePortalShowPlanned();
  const abort = useRef<AbortController | null>(null);

  const [selected, setSelected] = useState<string[]>([]);
  const [subject, setSubject] = useState<ReportSubjectKind>("people");
  const [granularity, setGranularity] = useState<ReportGranularity>("month");
  const [activeFamily, setActiveFamily] = useState<string | null>(null);
  const [preview, setPreview] = useState<PreviewState | null>(null);
  const [previewOpen, setPreviewOpen] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  const people = useMemo(() => scope.reportPeople ?? [], [scope.reportPeople]);
  const catalogue = useMemo(
    () =>
      (definitions.data?.metrics ?? []).filter(
        (metric) =>
          metric.is_enabled && metricVisible(metric.metric_key, showPlanned)
      ),
    [definitions.data, showPlanned]
  );
  const offered = useMemo<OfferedReportMetric[]>(
    () =>
      catalogue.map((metric) => ({
        ...metric,
        reason: metricReason(metric, subject),
      })),
    [catalogue, subject]
  );
  const families = useMemo(() => byFamily(offered), [offered]);
  const selectedMetrics = useMemo(
    () =>
      selected.filter((key) =>
        offered.some(
          (metric) => metric.metric_key === key && metric.reason == null
        )
      ),
    [offered, selected]
  );
  const recipe = useMemo<ReportRecipe>(
    () => ({
      subject:
        subject === "people"
          ? { type: "people", ids: people.map((person) => person.entityId) }
          : { type: "tenant" },
      period: dateRange,
      granularity,
      metric_keys: selectedMetrics,
    }),
    [dateRange, granularity, people, selectedMetrics, subject]
  );
  const recipeKey = JSON.stringify(recipe);
  const currentPreview =
    preview?.recipe === recipeKey ? preview.response : null;
  const running = previewRequest.isPending || exportRequest.isPending;
  const blocker = reportBlocker(
    subject,
    people.length,
    selected,
    selectedMetrics
  );

  useEffect(() => () => abort.current?.abort(), [recipeKey]);

  async function buildPreview(): Promise<void> {
    const controller = new AbortController();
    abort.current = controller;
    setFailure(null);
    setPreview(null);
    setPreviewOpen(false);

    try {
      const response = await previewRequest.mutateAsync({
        recipe,
        signal: controller.signal,
      });
      setPreview({ recipe: recipeKey, response });
      setPreviewOpen(true);
      for (const key of selectedMetrics) recordUsageEvent("report_column", key);
    } catch (error) {
      setFailure(
        controller.signal.aborted
          ? "Cancelled — no report was generated."
          : `Could not preview the report: ${(error as Error).message}`
      );
    } finally {
      if (abort.current === controller) abort.current = null;
    }
  }

  async function exportReport(format: "csv" | "xlsx"): Promise<void> {
    const controller = new AbortController();
    abort.current = controller;
    setFailure(null);

    try {
      await exportRequest.mutateAsync({
        recipe,
        format,
        signal: controller.signal,
      });
      recordUsageEvent("export", `report:${format}`);
    } catch (error) {
      setFailure(
        controller.signal.aborted
          ? "Cancelled — no report was downloaded."
          : `Could not export the report: ${(error as Error).message}`
      );
    } finally {
      if (abort.current === controller) abort.current = null;
    }
  }

  if (definitions.isError) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon
          variant="card"
          state="error"
          label="Unable to load the metric catalogue"
        />
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4 p-4 md:p-6">
      <div className="flex flex-col gap-1">
        <h1 className={TEXT_TITLE}>Report builder</h1>
        <p className="text-sm text-muted-foreground">
          {scope.label ? `${scope.label} · ` : ""}
          {subject === "people"
            ? `${people.length} ${people.length === 1 ? "person" : "people"} · one row per person per period`
            : "Tenant-wide metrics · one row per period"}
        </p>
      </div>

      <Card>
        <CardContent className="flex flex-col gap-4 p-4">
          <div className="flex flex-wrap items-center gap-3">
            <span className={TEXT_LABEL}>Subject</span>
            <ToggleGroup
              value={[subject]}
              onValueChange={(value) => {
                const next = Array.isArray(value) ? value[0] : value;
                if (next === "people" || next === "tenant") setSubject(next);
              }}
              variant="outline"
              size="sm"
            >
              <ToggleGroupItem value="people">People</ToggleGroupItem>
              <ToggleGroupItem value="tenant">Tenant</ToggleGroupItem>
            </ToggleGroup>
            <span className="text-xs text-muted-foreground">
              People reports use visible roster members. Tenant reports use
              tenant-wide metrics.
            </span>
          </div>

          <div className="flex flex-wrap items-center gap-3">
            <span className={TEXT_LABEL}>Rows</span>
            <ToggleGroup value={[subject]} variant="outline" size="sm">
              <ToggleGroupItem value={subject}>
                {subject === "people" ? "People" : "Tenant"}
              </ToggleGroupItem>
              <ToggleGroupItem value="repositories" disabled>
                Repositories
              </ToggleGroupItem>
            </ToggleGroup>
            <span className="text-xs text-muted-foreground">
              Repositories are not available for reports yet.
            </span>
          </div>

          <div className="flex flex-wrap items-center gap-3">
            <span className={TEXT_LABEL}>Granularity</span>
            <ToggleGroup
              value={[granularity]}
              onValueChange={(value) => {
                const next = Array.isArray(value) ? value[0] : value;
                if (isReportGranularity(next)) setGranularity(next);
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

          <div className="flex flex-col gap-3">
            <span className={TEXT_LABEL}>Metrics</span>
            {definitions.isPending ? (
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <Spinner className="size-4" /> Reading the catalogue…
              </div>
            ) : (
              <ReportMetricPicker
                families={families}
                activeFamily={activeFamily}
                selected={selected}
                setActiveFamily={setActiveFamily}
                setSelected={setSelected}
              />
            )}
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              disabled={blocker != null || running}
              onClick={() => void buildPreview()}
            >
              Preview report
            </Button>
            {blocker && !running ? (
              <span className="text-sm text-muted-foreground">{blocker}</span>
            ) : null}
            {running ? (
              <>
                <span className="text-sm text-muted-foreground">
                  Preparing report…
                </span>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => abort.current?.abort()}
                >
                  <X className="size-4" /> Cancel
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

      {currentPreview ? (
        <Button
          type="button"
          variant="outline"
          className="self-start"
          onClick={() => setPreviewOpen(true)}
        >
          <Download className="size-4" />
          {currentPreview.total_rows} rows · {granularity} · {dateRange.from} to{" "}
          {dateRange.to}
        </Button>
      ) : null}

      <ReportPreviewDialog
        response={currentPreview}
        open={previewOpen}
        period={dateRange}
        granularity={granularity}
        running={running}
        onOpenChange={setPreviewOpen}
        onExport={(format) => void exportReport(format)}
      />
    </div>
  );
}

function metricReason(
  metric: {
    entity_type: "person" | "tenant";
    schema_status: string;
    last_observed_date: string | null;
    origin: string;
  },
  subject: ReportSubjectKind
): string | null {
  if (metric.entity_type !== (subject === "people" ? "person" : "tenant")) {
    const reportKind =
      metric.entity_type === "person" ? "People" : "tenant-wide";
    return `This metric is available only in ${reportKind} reports`;
  }
  if (metric.schema_status === "error")
    return "This metric is not computing on this installation";
  if (metric.origin !== "custom" && metric.last_observed_date == null) {
    return "No data source is connected for this metric yet";
  }
  return null;
}

function reportBlocker(
  subject: ReportSubjectKind,
  peopleCount: number,
  selected: string[],
  selectedMetrics: string[]
): string | null {
  if (selectedMetrics.length === 0) {
    return selected.length > 0
      ? "Selected metrics are not available for this subject"
      : "Pick at least one metric";
  }
  if (subject === "people" && peopleCount === 0)
    return "This scope has no people";
  return null;
}

function isReportGranularity(
  value: string | undefined
): value is ReportGranularity {
  return (
    value === "day" ||
    value === "week" ||
    value === "month" ||
    value === "quarter" ||
    value === "year"
  );
}
