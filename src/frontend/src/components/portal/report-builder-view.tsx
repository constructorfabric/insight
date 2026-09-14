import { useEffect, useMemo, useRef, useState } from "react";

import type {
  ReportGranularity,
  ReportPreviewResponse,
  ReportRecipe,
} from "@/api/reports-client";
import { MAX_REPORT_PEOPLE } from "@/api/reports-client";
import { Card, CardContent } from "@/components/ui/card";
import { Spinner } from "@/components/ui/spinner";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { ReportBuilderActions } from "@/components/portal/report-builder-actions";
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
import {
  clampGranularity,
  periodTooShortReason,
} from "@/lib/reports/granularity-for-period";
import { TEXT_LABEL } from "@/lib/type-scale";
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
  const [pickedGranularity, setPickedGranularity] =
    useState<ReportGranularity>("month");
  const granularity = clampGranularity(pickedGranularity, dateRange);
  const [preview, setPreview] = useState<PreviewState | null>(null);
  const [previewOpen, setPreviewOpen] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  const people = useMemo(() => scope.roster ?? [], [scope.roster]);
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
      catalogue
        .filter(
          (metric) =>
            metric.entity_type === (subject === "people" ? "person" : "tenant")
        )
        .map((metric) => ({
          ...metric,
          reason: metricReason(metric),
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
          ? { type: "people", ids: people.map((person) => person.person_id) }
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
    <div className="flex flex-col gap-4 p-4 pb-24 md:p-6 md:pb-24">
      <Card className="gap-0 py-0">
        <CardContent className="flex flex-col p-0">
          <div className="flex flex-wrap items-center gap-x-6 gap-y-3 border-b p-4">
            <div className="flex flex-wrap items-center gap-3">
              <span className={TEXT_LABEL}>Scope</span>
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
            </div>

            <div className="hidden h-6 border-l md:block" />

            <div className="flex flex-wrap items-center gap-3">
              <span className={TEXT_LABEL}>Granularity</span>
              <TooltipProvider delay={300}>
                <ToggleGroup
                  value={[granularity]}
                  onValueChange={(value) => {
                    const next = Array.isArray(value) ? value[0] : value;
                    if (isReportGranularity(next)) setPickedGranularity(next);
                  }}
                  variant="outline"
                  size="sm"
                >
                  {GRANULARITIES.map((option) => {
                    const reason = periodTooShortReason(option.value, dateRange);
                    return (
                      <Tooltip key={option.value}>
                        <TooltipTrigger
                          render={
                            <ToggleGroupItem
                              value={option.value}
                              disabled={reason != null}
                              title={reason ?? undefined}
                            >
                              {option.label}
                            </ToggleGroupItem>
                          }
                        />
                        {reason ? (
                          <TooltipContent
                            side="top"
                            className="max-w-xs text-xs leading-relaxed"
                          >
                            {reason}
                          </TooltipContent>
                        ) : null}
                      </Tooltip>
                    );
                  })}
                </ToggleGroup>
              </TooltipProvider>
            </div>
          </div>

          {definitions.isPending ? (
            <div className="flex items-center gap-2 p-4 text-sm text-muted-foreground">
              <Spinner className="size-4" /> Reading the catalogue…
            </div>
          ) : (
            <ReportMetricPicker
              families={families}
              selected={selected}
              setSelected={setSelected}
            />
          )}
        </CardContent>
      </Card>

      <ReportPreviewDialog
        response={currentPreview}
        open={previewOpen}
        period={dateRange}
        granularity={granularity}
        running={running}
        onOpenChange={setPreviewOpen}
        onExport={(format) => void exportReport(format)}
      />

      <ReportBuilderActions
        selectedCount={selectedMetrics.length}
        hasSelection={selected.length > 0}
        blocker={blocker}
        running={running}
        failure={failure}
        onClear={() => setSelected([])}
        onCancel={() => abort.current?.abort()}
        onPreview={() => void buildPreview()}
      />
    </div>
  );
}

function metricReason(metric: {
  schema_status: string;
  last_observed_date: string | null;
  origin: string;
}): string | null {
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
  if (subject === "people" && peopleCount > MAX_REPORT_PEOPLE) {
    return `This scope has ${peopleCount.toLocaleString()} people; reports support up to ${MAX_REPORT_PEOPLE.toLocaleString()}. Narrow the scope`;
  }
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
