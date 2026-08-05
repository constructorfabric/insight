import { useMemo, useState } from "react";
import { Database, ListFilter, X } from "lucide-react";

import { evidenceSelection } from "@/api/metric-drilldown-client";
import type { DateRange } from "@/api/period-to-date-range";
import {
  useMetricEvidenceOptional,
  type EvidenceDialogTarget,
} from "@/components/metric-evidence-context";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Popover,
  PopoverContent,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import {
  TimeseriesBody,
  TimeseriesExportMenu,
  TimeseriesPresentationToggle,
} from "@/components/widgets/metric-views/metric-timeseries-chrome";
import { shouldCombineTimeseriesMetrics } from "@/components/widgets/metric-views/metric-timeseries-chart-model";
import {
  parseTimeseriesPresentation,
  serializeTimeseriesPresentation,
  type TimeseriesPresentation,
} from "@/components/widgets/metric-views/metric-timeseries-presentation";
import { buildMetricTimeseriesModel } from "@/components/widgets/metric-views/metric-timeseries-model";
import {
  forEntity,
  resolveTimeseriesBucket,
  type MetricCollectionConfig,
  type MetricTimeseriesGroupLimitConfig,
} from "@/lib/metrics/collection";
import type { MetricTimeseriesTableConfig } from "@/lib/metrics/timeseries-table";
import type { MetricTimeseriesChartConfig } from "@/lib/metrics/timeseries-chart";
import { cn } from "@/lib/utils";
import { useLocalStorageState } from "@/hooks/use-local-storage-state";
import {
  useMetricCollection,
  useMetricCollectionSet,
} from "@/queries/metric-results";

interface MetricTimeseriesGroupBy {
  default: string;
  options?: string[];
  limits?: Record<string, MetricTimeseriesGroupLimitConfig>;
}

interface DimensionFilterControl {
  dimension: string;
  options: Array<{ value: string; label: string }>;
  selectedValues: string[];
  selectedLabels: string[];
  disabled: boolean;
}

interface DimensionControlsProps {
  dimensions: string[];
  selectedDimension: string;
  filters: DimensionFilterControl[];
  onDimensionChange: (dimension: string) => void;
  onFilterChange: (dimension: string, values: string[]) => void;
  className?: string;
}

export interface MetricTimeseriesViewProps {
  id: string;
  entityId: string;
  range: DateRange;
  metricKeys: string[];
  defaultPresentation?: Presentation;
  chart?: MetricTimeseriesChartConfig;
  groupBy?: MetricTimeseriesGroupBy;
  table?: MetricTimeseriesTableConfig;
}

type Presentation = TimeseriesPresentation;

function dimensionName(dimension: string): string {
  const label = dimension.replaceAll("_", " ");
  return label.charAt(0).toUpperCase() + label.slice(1);
}

function dimensionDescription(dimension: string): string {
  return dimension.replaceAll("_", " ");
}

function DimensionControls({
  dimensions,
  selectedDimension,
  filters,
  onDimensionChange,
  onFilterChange,
  className,
}: DimensionControlsProps) {
  return (
    <div className={cn("flex flex-wrap items-center gap-2", className)}>
      {dimensions.length > 1 ? (
        <ToggleGroup
          value={[selectedDimension]}
          onValueChange={(value) => {
            const next = Array.isArray(value) ? value[0] : value;
            if (next) onDimensionChange(next);
          }}
          variant="outline"
          size="sm"
          aria-label="Group by"
        >
          {dimensions.map((dimension) => (
            <ToggleGroupItem key={dimension} value={dimension}>
              {dimensionName(dimension)}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      ) : null}
      {filters.length > 0 ? (
        <Popover>
          <PopoverTrigger
            render={
              <Button
                type="button"
                variant="outline"
                size="icon-sm"
                aria-label="Filters"
                title="Filters"
              >
                <ListFilter className="size-4" />
              </Button>
            }
          />
          <PopoverContent align="start" className="w-72">
            <PopoverHeader>
              <PopoverTitle>Filters</PopoverTitle>
            </PopoverHeader>
            {filters.map((filter) => (
              <div key={filter.dimension} className="flex flex-col gap-2">
                <div className="flex items-center justify-between gap-2">
                  <span className="text-xs font-medium">
                    {dimensionName(filter.dimension)}
                  </span>
                  {filter.selectedValues.length > 0 ? (
                    <Button
                      type="button"
                      variant="ghost"
                      size="xs"
                      onClick={() => onFilterChange(filter.dimension, [])}
                    >
                      Clear
                    </Button>
                  ) : null}
                </div>
                <div className="max-h-56 space-y-1 overflow-y-auto">
                  {filter.options.map((option) => {
                    const checked = filter.selectedValues.includes(
                      option.value
                    );
                    return (
                      <label
                        key={option.value}
                        htmlFor={`filter-${filter.dimension}-${option.value}`}
                        className="flex cursor-pointer items-center gap-2 rounded-sm px-1 py-1 text-sm hover:bg-muted"
                      >
                        <Checkbox
                          id={`filter-${filter.dimension}-${option.value}`}
                          checked={checked}
                          disabled={filter.disabled}
                          onCheckedChange={() => {
                            onFilterChange(
                              filter.dimension,
                              checked
                                ? filter.selectedValues.filter(
                                    (value) => value !== option.value
                                  )
                                : [...filter.selectedValues, option.value]
                            );
                          }}
                        />
                        {option.label}
                      </label>
                    );
                  })}
                </div>
              </div>
            ))}
          </PopoverContent>
        </Popover>
      ) : null}
      {filters
        .filter((filter) => filter.selectedValues.length > 0)
        .map((filter) => (
          <Button
            key={filter.dimension}
            type="button"
            variant="secondary"
            size="xs"
            className="rounded-full"
            aria-label={`Clear ${dimensionName(filter.dimension)} filter`}
            onClick={() => onFilterChange(filter.dimension, [])}
          >
            {dimensionName(filter.dimension)}:{" "}
            {filter.selectedLabels.length === 1
              ? filter.selectedLabels[0]
              : `${filter.selectedLabels.length} selected`}
            <X className="size-3" />
          </Button>
        ))}
    </div>
  );
}

export function MetricTimeseriesView({
  id,
  entityId,
  range,
  metricKeys,
  defaultPresentation = "chart",
  chart,
  groupBy,
  table,
}: MetricTimeseriesViewProps) {
  const evidenceContext = useMetricEvidenceOptional();
  const [presentation, setPresentation] = useLocalStorageState<Presentation>({
    key: `insight.timeseries.${id}.presentation`,
    defaultValue: defaultPresentation,
    parse: parseTimeseriesPresentation,
    serialize: serializeTimeseriesPresentation,
  });
  const [selectedMetricKey, setSelectedMetricKey] = useState(
    metricKeys[0] ?? ""
  );
  const dimensionOptions = useMemo(
    () =>
      groupBy
        ? [...new Set([groupBy.default, ...(groupBy.options ?? [])])]
        : [],
    [groupBy]
  );
  const [selectedGroupBy, setSelectedGroupBy] = useState(
    groupBy?.default ?? ""
  );
  const [dimensionFilters, setDimensionFilters] = useState<
    Record<string, string[]>
  >({});
  const filters = useMemo(
    () =>
      Object.entries(dimensionFilters)
        .sort(([left], [right]) => left.localeCompare(right))
        .flatMap(([dimension, values]) =>
          values.length > 0 ? [{ dimension, values }] : []
        ),
    [dimensionFilters]
  );
  const groupLimit = selectedGroupBy
    ? groupBy?.limits?.[selectedGroupBy]
    : undefined;
  const collection = useMemo<MetricCollectionConfig>(
    () => ({
      metrics: metricKeys.map((key) => ({
        key,
        filters,
        views: [
          {
            view: "timeseries",
            bucket: resolveTimeseriesBucket(range),
            dimensions: selectedGroupBy ? [selectedGroupBy] : [],
            ...(groupLimit
              ? {
                  groupLimit: {
                    count: groupLimit.count,
                    rank_by_metric: groupLimit.rankBy,
                    include_remainder: groupLimit.includeRemainder,
                  },
                }
              : {}),
          },
          { view: "period" },
        ],
      })),
    }),
    [filters, groupLimit, metricKeys, range, selectedGroupBy]
  );
  const entity = useMemo(
    () => ({ type: "person" as const, ids: [entityId] }),
    [entityId]
  );
  const data = useMetricCollection(collection, entity, range, {
    keepPreviousData: true,
  });
  const optionCollections = useMemo(
    () =>
      selectedMetricKey && dimensionOptions.length > 1
        ? dimensionOptions
            .filter((dimension) => dimension !== selectedGroupBy)
            .map((dimension) => ({
              key: dimension,
              collection: {
                metrics: [
                  {
                    key: selectedMetricKey,
                    views: [
                      { view: "breakdown" as const, dimensions: [dimension] },
                    ],
                  },
                ],
              },
            }))
        : [],
    [dimensionOptions, selectedGroupBy, selectedMetricKey]
  );
  const optionData = useMetricCollectionSet(optionCollections, entity, range);

  const model = buildMetricTimeseriesModel(
    data.byKey,
    metricKeys,
    entityId,
    range,
    selectedGroupBy ? [selectedGroupBy] : []
  );
  const empty = model.metrics.length === 0 || model.columns.length === 0;
  const selectedMetric =
    model.metrics.find((metric) => metric.metric_key === selectedMetricKey) ??
    model.metrics[0];
  const shouldCombineMetrics =
    presentation === "chart" &&
    shouldCombineTimeseriesMetrics(model, chart?.multiMetric ?? "selectable");
  const evidenceMetrics =
    presentation === "table" || shouldCombineMetrics
      ? model.metrics
      : selectedMetric
        ? [selectedMetric]
        : [];
  const evidenceTargets = evidenceMetrics.flatMap<EvidenceDialogTarget>(
    (metric) => {
      if (!metric.drilldown) return [];
      const selection = evidenceSelection(
        metric.selection,
        entityId,
        range,
        filters,
        metric.computation !== "ratio" && selectedGroupBy
          ? [selectedGroupBy]
          : []
      );
      return selection ? [{ selection, label: metric.label }] : [];
    }
  );
  const filterModels = dimensionOptions
    .filter((dimension) => dimension !== selectedGroupBy)
    .map((dimension) => {
      const result = optionData.get(dimension);
      const metric = result?.byKey.get(selectedMetricKey);
      const values = new Map<string, string>();
      if (metric) {
        for (const row of forEntity(metric, entityId).breakdown) {
          const item = row.dimensions.find(
            (candidate) => candidate.key === dimension
          );
          if (item) values.set(item.value, item.label ?? item.value);
        }
      }
      const options = [...values]
        .map(([value, label]) => ({ value, label }))
        .sort((left, right) => left.label.localeCompare(right.label));
      const selectedValues = dimensionFilters[dimension] ?? [];
      return {
        dimension,
        options,
        selectedValues,
        selectedLabels: selectedValues.map(
          (value) => values.get(value) ?? value
        ),
        disabled: Boolean(result?.isPending || result?.isError),
      };
    });
  const displayBucket =
    model.metrics.length > 0 ? model.bucket : resolveTimeseriesBucket(range);
  const bucketLabel =
    displayBucket === "day"
      ? "Daily"
      : displayBucket === "week"
        ? "Weekly"
        : "Monthly";

  function changeDimension(dimension: string): void {
    if (!dimension) return;
    setSelectedGroupBy(dimension);
    setDimensionFilters((current) => {
      const next = { ...current };
      delete next[dimension];
      return next;
    });
  }

  function changeFilter(dimension: string, values: string[]): void {
    setDimensionFilters((current) => {
      const next = { ...current };
      const normalized = [...new Set(values)].sort();
      if (normalized.length > 0) next[dimension] = normalized;
      else delete next[dimension];
      return next;
    });
  }

  function openTimeseriesEvidence(
    metricKey: string,
    columnKey: string,
    bucketStart: string | null
  ): void {
    const metric = model.metrics.find(
      (candidate) => candidate.metric_key === metricKey
    );
    const column = model.columns.find(
      (candidate) => candidate.key === columnKey
    );
    if (!metric?.drilldown || !column || column.remainder) return;
    const exactFilters = new Map(
      filters.map((filter) => [filter.dimension, filter])
    );
    for (const dimension of column.dimensions ?? []) {
      exactFilters.set(dimension.key, {
        dimension: dimension.key,
        values: [dimension.value],
      });
    }
    let period = range;
    if (bucketStart) {
      const index = model.buckets.indexOf(bucketStart);
      const next = model.buckets[index + 1];
      const to = next ? new Date(`${next}T00:00:00Z`) : null;
      if (to) to.setUTCDate(to.getUTCDate() - 1);
      period = {
        from: bucketStart < range.from ? range.from : bucketStart,
        to: to
          ? to.toISOString().slice(0, 10) > range.to
            ? range.to
            : to.toISOString().slice(0, 10)
          : range.to,
      };
    }
    const selection = evidenceSelection(
      metric.selection,
      entityId,
      period,
      [...exactFilters.values()].sort((left, right) =>
        left.dimension.localeCompare(right.dimension)
      ),
      metric.computation !== "ratio" && selectedGroupBy ? [selectedGroupBy] : []
    );
    if (selection) evidenceContext?.openEvidence(selection, metric.label);
  }

  return (
    <Card
      className={cn(
        "shrink-0 gap-0 overflow-hidden py-0",
        data.isFetching && "opacity-60"
      )}
    >
      <div className="flex items-center justify-between gap-2 border-b p-2">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          {selectedMetric ? (
            shouldCombineMetrics ? (
              <h3 className="px-2 text-sm font-semibold">
                {model.metrics.map((metric) => metric.label).join(" & ")}
              </h3>
            ) : model.metrics.length > 1 && presentation === "chart" ? (
              <Select
                value={selectedMetric.metric_key}
                onValueChange={(value) => {
                  if (value) setSelectedMetricKey(value);
                }}
              >
                <SelectTrigger
                  size="sm"
                  aria-label="Metric"
                  className="border-transparent bg-transparent ps-2 pe-2 font-semibold shadow-none hover:bg-muted/50 focus-visible:border-transparent focus-visible:ring-2 focus-visible:ring-ring/40 data-popup-open:bg-muted/50 dark:bg-transparent dark:hover:bg-muted/50"
                >
                  <SelectValue>{selectedMetric.label}</SelectValue>
                </SelectTrigger>
                <SelectContent align="start">
                  {model.metrics.map((metric) => (
                    <SelectItem
                      key={metric.metric_key}
                      value={metric.metric_key}
                    >
                      {metric.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            ) : model.metrics.length === 1 ? (
              <h3 className="px-2 text-sm font-semibold">
                {selectedMetric.label}
              </h3>
            ) : null
          ) : null}
        </div>
        <div className="flex shrink-0 items-center justify-end gap-2">
          {dimensionOptions.length > 1 || filterModels.length > 0 ? (
            <DimensionControls
              dimensions={dimensionOptions}
              selectedDimension={selectedGroupBy}
              filters={filterModels}
              onDimensionChange={changeDimension}
              onFilterChange={changeFilter}
            />
          ) : null}
          {evidenceContext && evidenceTargets.length > 0 ? (
            <Button
              type="button"
              variant="outline"
              size="icon-sm"
              disabled={data.isFetching}
              aria-label="View supporting data"
              title="View supporting data"
              onClick={() =>
                evidenceContext?.openEvidenceTargets(
                  evidenceTargets,
                  evidenceTargets.map((target) => target.label).join(" & ")
                )
              }
            >
              <Database className="size-4" />
            </Button>
          ) : null}
          <TimeseriesExportMenu
            id={id}
            model={model}
            range={range}
            disabled={empty || data.isFetching || data.isError}
          />
          <TimeseriesPresentationToggle
            presentation={presentation}
            onChange={setPresentation}
          />
        </div>
      </div>
      <CardContent
        className="relative flex h-96 min-h-0 flex-col px-0"
        aria-busy={data.isFetching}
      >
        {presentation === "chart" ? (
          <div className="min-h-10 shrink-0 px-4 py-2 text-xs text-muted-foreground sm:px-6">
            {selectedGroupBy
              ? `${bucketLabel} by ${dimensionDescription(selectedGroupBy)}`
              : bucketLabel}
          </div>
        ) : null}
        <TimeseriesBody
          isPending={data.isPending}
          isFetching={data.isFetching}
          isError={data.isError}
          onRetry={data.refetch}
          empty={empty}
          presentation={presentation}
          model={model}
          selectedMetricKey={selectedMetric?.metric_key ?? ""}
          multiMetric={shouldCombineMetrics ? "combined" : "selectable"}
          table={table}
          onEvidence={openTimeseriesEvidence}
        />
      </CardContent>
    </Card>
  );
}
