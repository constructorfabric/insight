import { Info, Search, TriangleAlert } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import type { MetricDefinition } from "@/api/metric-definitions-client";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Input } from "@/components/ui/input";
import { SidebarTrigger } from "@/components/ui/sidebar";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { formatDate } from "@/lib/format";
import {
  useMetricDefinitions,
  type MetricDefinitionGroup,
} from "@/queries/metric-definitions";

export function MetricDefinitionsScreen() {
  const { t } = useTranslation();
  const { data: groups, isPending, isError } = useMetricDefinitions();
  const [query, setQuery] = useState("");

  const filtered = useMemo(
    () => filterGroups(groups ?? [], query),
    [groups, query]
  );
  const hasResults = filtered.some((group) => group.metrics.length > 0);

  return (
    <>
      <header className="sticky top-0 z-20 flex flex-wrap items-center gap-3 border-b bg-background/95 px-4 py-3 backdrop-blur-sm">
        <SidebarTrigger />
        <h1 className="text-xl font-semibold tracking-tight">
          {t("metric_definitions.title")}
        </h1>
        <div className="relative ms-auto w-full max-w-xs">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t("metric_definitions.search_placeholder")}
            className="ps-8"
            aria-label={t("metric_definitions.search_placeholder")}
          />
        </div>
      </header>

      <main className="flex flex-1 flex-col p-4 md:p-6">
        <div className="mx-auto flex w-full max-w-5xl flex-col gap-6 pb-12">
          {isPending ? <CenteredSpinner className="min-h-[70vh]" /> : null}

          {isError ? (
            <Alert variant="destructive">
              <TriangleAlert />
              <AlertTitle>{t("metric_definitions.error_title")}</AlertTitle>
              <AlertDescription>
                {t("metric_definitions.error_description")}
              </AlertDescription>
            </Alert>
          ) : null}

          {!isPending && !isError && !hasResults ? (
            <Empty className="min-h-[50vh]">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <Search />
                </EmptyMedia>
                <EmptyTitle>
                  {t("metric_definitions.no_results_title")}
                </EmptyTitle>
                <EmptyDescription>
                  {t("metric_definitions.no_results_description", { query })}
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
          ) : null}

          {filtered.map((group) =>
            group.metrics.length > 0 ? (
              <MetricGroupCard key={group.prefix} group={group} />
            ) : null
          )}
        </div>
      </main>
    </>
  );
}

function MetricGroupCard({ group }: { group: MetricDefinitionGroup }) {
  const { t } = useTranslation();

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <span className="font-mono text-sm tracking-wide uppercase">
            {group.prefix}
          </span>
          <Badge variant="secondary">{group.metrics.length}</Badge>
        </CardTitle>
      </CardHeader>
      <CardContent className="overflow-x-auto">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("metric_definitions.columns.metric")}</TableHead>
              <TableHead>
                {t("metric_definitions.columns.description")}
              </TableHead>
              <TableHead>{t("metric_definitions.columns.format")}</TableHead>
              <TableHead>
                {t("metric_definitions.columns.better_when")}
              </TableHead>
              <TableHead>
                {t("metric_definitions.columns.dimensions")}
              </TableHead>
              <TableHead>{t("metric_definitions.columns.status")}</TableHead>
              <TableHead>
                <span className="inline-flex items-center gap-1.5">
                  {t("metric_definitions.columns.last_data")}
                  <TooltipProvider>
                    <Tooltip>
                      <TooltipTrigger
                        render={
                          <span className="inline-flex text-muted-foreground" />
                        }
                      >
                        <Info className="size-3.5" />
                      </TooltipTrigger>
                      <TooltipContent className="max-w-xs">
                        {t("metric_definitions.last_data_help")}
                      </TooltipContent>
                    </Tooltip>
                  </TooltipProvider>
                </span>
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {group.metrics.map((metric) => (
              <MetricRow key={metric.metric_key} metric={metric} />
            ))}
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  );
}

function MetricRow({ metric }: { metric: MetricDefinition }) {
  const { t } = useTranslation();
  const showShort =
    metric.short_label != null && metric.short_label !== metric.label;

  return (
    <TableRow className={metric.is_enabled ? undefined : "opacity-60"}>
      <TableCell className="align-top">
        <div className="flex items-center gap-1.5 font-medium">
          {metric.label}
          {showShort ? (
            <span className="text-xs font-normal text-muted-foreground">
              ({metric.short_label})
            </span>
          ) : null}
          {metric.explanation ? (
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger
                  render={
                    <span className="inline-flex text-muted-foreground" />
                  }
                >
                  <Info className="size-3.5" />
                </TooltipTrigger>
                <TooltipContent className="max-w-xs">
                  {metric.explanation}
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          ) : null}
        </div>
        <div className="font-mono text-xs text-muted-foreground">
          {metric.metric_key}
        </div>
      </TableCell>
      <TableCell className="max-w-md align-top whitespace-normal text-muted-foreground">
        {metric.description}
      </TableCell>
      <TableCell className="align-top">
        <Badge variant="outline">
          {t(`metric_definitions.format.${metric.format}`)}
          {metric.unit ? ` · ${metric.unit}` : null}
        </Badge>
      </TableCell>
      <TableCell className="align-top whitespace-nowrap text-muted-foreground">
        {t(`metric_definitions.better_when.${metric.direction}`)}
      </TableCell>
      <TableCell className="align-top">
        <div className="flex flex-wrap gap-1">
          {metric.dimensions.map((dimension) => (
            <Badge key={dimension} variant="secondary" className="font-mono">
              {dimension}
            </Badge>
          ))}
        </div>
      </TableCell>
      <TableCell className="align-top">
        <StatusBadge metric={metric} />
      </TableCell>
      <TableCell className="align-top whitespace-nowrap text-muted-foreground">
        {formatLastObserved(metric.last_observed_date)}
      </TableCell>
    </TableRow>
  );
}

function StatusBadge({ metric }: { metric: MetricDefinition }) {
  const { t } = useTranslation();

  if (!metric.is_enabled) {
    return (
      <Badge variant="outline">{t("metric_definitions.status.disabled")}</Badge>
    );
  }
  if (metric.schema_status === "error") {
    const reason = metric.schema_error_code
      ? t(`metric_definitions.error_code.${metric.schema_error_code}`)
      : null;
    const badge = (
      <Badge variant="destructive">
        {t("metric_definitions.status.schema_error")}
      </Badge>
    );
    if (!reason) return badge;
    return (
      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger render={<span className="inline-flex" />}>
            {badge}
          </TooltipTrigger>
          <TooltipContent className="max-w-xs">{reason}</TooltipContent>
        </Tooltip>
      </TooltipProvider>
    );
  }
  if (metric.schema_status === "unchecked") {
    return (
      <Badge variant="secondary">
        {t("metric_definitions.status.unchecked")}
      </Badge>
    );
  }
  return (
    <Badge variant="secondary">{t("metric_definitions.status.active")}</Badge>
  );
}

function formatLastObserved(date: string | null): string {
  if (!date) return "—";
  return formatDate(date, "d MMM yyyy");
}

function filterGroups(
  groups: MetricDefinitionGroup[],
  query: string
): MetricDefinitionGroup[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return groups;
  return groups.map((group) => ({
    ...group,
    metrics: group.metrics.filter((metric) => matchesQuery(metric, needle)),
  }));
}

function matchesQuery(metric: MetricDefinition, needle: string): boolean {
  return [
    metric.metric_key,
    metric.label,
    metric.short_label,
    metric.description,
  ].some((field) => field?.toLowerCase().includes(needle));
}
