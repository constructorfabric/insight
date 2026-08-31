import { useMemo, useState, type Dispatch, type SetStateAction } from "react";
import { Search } from "lucide-react";

import type { MetricDefinition } from "@/api/metric-definitions-client";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { MetricFamily } from "@/lib/reports/families";
import { cn } from "@/lib/utils";
import { TEXT_EYEBROW } from "@/lib/type-scale";

import styles from "./report-builder-metrics.module.css";

const ALL_FAMILIES = "all";

export interface OfferedReportMetric extends MetricDefinition {
  reason: string | null;
}

interface ReportMetricPickerProps {
  families: MetricFamily<OfferedReportMetric>[];
  selected: string[];
  setSelected: Dispatch<SetStateAction<string[]>>;
}

export function ReportMetricPicker({
  families,
  selected,
  setSelected,
}: ReportMetricPickerProps) {
  const [query, setQuery] = useState("");
  const [pickedFamily, setPickedFamily] = useState(ALL_FAMILIES);
  const activeFamily =
    pickedFamily === ALL_FAMILIES ||
    families.some((family) => family.family === pickedFamily)
      ? pickedFamily
      : ALL_FAMILIES;
  const visibleFamilies = useMemo(() => {
    const normalizedQuery = query.trim().toLocaleLowerCase();
    return families
      .filter(
        (family) =>
          activeFamily === ALL_FAMILIES || family.family === activeFamily
      )
      .map((family) => ({
        family,
        metrics: family.metrics.filter((metric) =>
          matchesQuery(metric, normalizedQuery)
        ),
      }))
      .filter(({ metrics }) => metrics.length > 0);
  }, [activeFamily, families, query]);

  if (families.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        No metrics are available for this scope.
      </p>
    );
  }

  return (
    <TooltipProvider delay={300}>
      <div className="flex flex-col">
        <div className="flex flex-col gap-3 pt-3">
          <div className="px-4">
            <div className="relative">
              <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                type="search"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="Search metrics"
                aria-label="Search metrics"
                className="ps-8"
              />
            </div>
          </div>
          <Tabs
            value={activeFamily}
            onValueChange={(value) => setPickedFamily(String(value))}
          >
            <div className="flex overflow-x-auto overflow-y-hidden border-b px-4">
              <TabsList
                variant="line"
                className={cn("w-max", styles.tabsList)}
              >
                <FamilyTab
                  value={ALL_FAMILIES}
                  label="All metrics"
                  families={families}
                  selected={selected}
                />
                {families.map((family) => (
                  <FamilyTab
                    key={family.family}
                    value={family.family}
                    label={familyTabLabel(family.name)}
                    families={[family]}
                    selected={selected}
                  />
                ))}
              </TabsList>
            </div>
          </Tabs>
        </div>

        <div className="flex flex-col gap-5 p-4">
          {visibleFamilies.map(({ family, metrics }) => (
            <MetricFamilyGroup
              key={family.family}
              family={family}
              visibleMetrics={metrics}
              selected={selected}
              setSelected={setSelected}
            />
          ))}
          {visibleFamilies.length === 0 ? (
            <p className="py-10 text-center text-sm text-muted-foreground">
              No metrics match this search.
            </p>
          ) : null}
        </div>
      </div>
    </TooltipProvider>
  );
}

function FamilyTab({
  value,
  label,
  families,
  selected,
}: {
  value: string;
  label: string;
  families: MetricFamily<OfferedReportMetric>[];
  selected: string[];
}) {
  const metrics = families.flatMap((family) => family.metrics);
  const selectedCount = metrics.filter((metric) =>
    selected.includes(metric.metric_key)
  ).length;
  const count = selectedCount
    ? `${selectedCount}/${metrics.length}`
    : String(metrics.length);

  return (
    <TabsTrigger value={value}>
      {label}
      <span className="rounded-full bg-muted px-1.5 text-xs text-muted-foreground tabular-nums">
        {count}
      </span>
    </TabsTrigger>
  );
}

function MetricFamilyGroup({
  family,
  visibleMetrics,
  selected,
  setSelected,
}: {
  family: MetricFamily<OfferedReportMetric>;
  visibleMetrics: OfferedReportMetric[];
  selected: string[];
  setSelected: Dispatch<SetStateAction<string[]>>;
}) {
  const selectable = family.metrics
    .filter((metric) => metric.reason == null)
    .map((metric) => metric.metric_key);
  const selectedCount = family.metrics.filter((metric) =>
    selected.includes(metric.metric_key)
  ).length;
  const allPicked =
    selectable.length > 0 && selectable.every((key) => selected.includes(key));

  return (
    <section className="flex flex-col gap-2">
      <div className="flex items-center gap-3">
        <h2 className={TEXT_EYEBROW}>{family.name}</h2>
        <Separator className="flex-1" />
        <span className="text-xs text-muted-foreground tabular-nums">
          {selectedCount > 0
            ? `${selectedCount} of ${family.metrics.length} selected`
            : `${family.metrics.length} metrics`}
        </span>
        {selectable.length > 0 ? (
          <Button
            type="button"
            variant="link"
            size="xs"
            onClick={() =>
              setSelected((current) =>
                allPicked
                  ? current.filter((key) => !selectable.includes(key))
                  : [...new Set([...current, ...selectable])]
              )
            }
          >
            {allPicked ? "Clear all" : "Select all"}
          </Button>
        ) : null}
      </div>
      <div className="grid grid-cols-1 gap-x-4 gap-y-0.5 sm:grid-cols-2 xl:grid-cols-3">
        {visibleMetrics.map((metric) => {
          const checked = selected.includes(metric.metric_key);
          return (
            <Tooltip key={metric.metric_key}>
              <TooltipTrigger
                render={
                  <label
                    htmlFor={`report-${metric.metric_key}`}
                    title={metric.reason ?? undefined}
                    className={cn(
                      "flex min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-start text-sm",
                      metric.reason
                        ? "text-muted-foreground"
                        : "cursor-pointer hover:bg-muted",
                      checked && "bg-primary/5 font-medium"
                    )}
                  >
                    <Checkbox
                      id={`report-${metric.metric_key}`}
                      checked={checked}
                      disabled={metric.reason != null}
                      onCheckedChange={() =>
                        setSelected((current) =>
                          checked
                            ? current.filter((key) => key !== metric.metric_key)
                            : [...current, metric.metric_key]
                        )
                      }
                    />
                    <span className="min-w-0 flex-1 truncate">
                      {metric.label}
                    </span>
                    <span
                      aria-hidden="true"
                      className="shrink-0 text-xs text-muted-foreground uppercase"
                    >
                      {metricTag(metric)}
                    </span>
                  </label>
                }
              />
              <TooltipContent
                side="top"
                className="max-w-xs text-xs leading-relaxed"
              >
                {metric.reason ??
                  metric.description ??
                  metric.explanation ??
                  metric.label}
              </TooltipContent>
            </Tooltip>
          );
        })}
      </div>
    </section>
  );
}

function matchesQuery(
  metric: OfferedReportMetric,
  normalizedQuery: string
): boolean {
  if (!normalizedQuery) return true;
  return [
    metric.metric_key,
    metric.label,
    metric.short_label,
    metric.description,
    metric.explanation,
    metric.unit,
    metric.format,
  ].some((value) => value?.toLocaleLowerCase().includes(normalizedQuery));
}

function familyTabLabel(name: string): string {
  return name.replace(/^Development · /, "");
}

function metricTag(metric: OfferedReportMetric): string {
  return metric.unit ?? metric.format;
}
