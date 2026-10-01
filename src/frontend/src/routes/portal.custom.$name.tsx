import { createFileRoute, useRouterState } from "@tanstack/react-router";
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Table2 } from "lucide-react";

import { Button } from "@/components/ui/button";

import {
  CustomApiError,
  type Dashboard,
  type RunOptions,
} from "@/api/custom-client";
import { DashboardPageActions } from "@/components/custom/dashboard-page-actions";
import { RangePicker } from "@/components/custom/range-picker";
import { selectedRange } from "@/lib/custom/board-range";
import { dashboardNameFromPath } from "@/lib/custom/dashboard-path";
import { drawsBucket } from "@/lib/custom/draws-bucket";
import {
  useSetPortalSearch,
  usePortalSearch,
} from "@/lib/portal/portal-search";
import { dashboardItems } from "@/lib/custom/dashboard-items";
import { WidgetFrame } from "@/components/charts/widget-frame";
import { CustomWidget } from "@/components/custom/custom-widget";
import { refusal } from "@/components/custom/refusal";
import { WidgetDrilldown } from "@/components/custom/widget-drilldown";
import { Badge } from "@/components/ui/badge";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import {
  dashboardQuery,
  metricQuery,
  metricResultQuery,
  widgetQuery,
} from "@/queries/custom";
import { TEXT_BODY, TEXT_TITLE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export const Route = createFileRoute("/portal/custom/$name")({
  component: CustomDashboardPage,
});

function CustomDashboardPage() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const name = dashboardNameFromPath(pathname);
  const search = usePortalSearch();
  const setSearch = useSetPortalSearch();
  const {
    data: dashboard,
    isLoading,
    isError,
    error,
    refetch,
  } = useQuery(dashboardQuery(name));

  const offered = dashboard?.time_ranges;
  const range = selectedRange(offered, dashboard?.default_range, search.range);
  return (
    <CustomDashboardBody
      dashboard={dashboard}
      isLoading={isLoading}
      isError={isError}
      error={error}
      name={name}
      onRetry={() => void refetch()}
      range={range}
      offered={offered}
      onSelectRange={(token) => setSearch({ range: token })}
    />
  );
}

function CustomDashboardBody({
  dashboard,
  isLoading,
  isError,
  error,
  name,
  onRetry,
  range,
  offered,
  onSelectRange,
}: {
  dashboard: Dashboard | undefined;
  isLoading: boolean;
  isError: boolean;
  error: Error | null;
  name: string;
  onRetry: () => void;
  range: string | undefined;
  offered: string[] | undefined;
  onSelectRange: (token: string) => void;
}) {
  if (isLoading) return <CenteredSpinner className="min-h-40" />;
  if (isError) {
    if (error instanceof CustomApiError && error.status === 404) {
      return (
        <ComingSoon
          variant="card"
          state="empty"
          label={`No dashboard named "${name}".`}
        />
      );
    }
    return (
      <ComingSoon
        variant="card"
        state="error"
        label="Couldn't load this dashboard."
        onRetry={onRetry}
      />
    );
  }
  if (!dashboard) return null;

  const items = dashboardItems(dashboard);

  return (
    <>
      <header className="sticky -top-4 z-10 -mx-4 -mt-4 mb-4 flex flex-wrap items-center justify-between gap-x-3 gap-y-2 border-b bg-background px-4 pt-4 pb-3 md:-top-6 md:-mx-6 md:-mt-6 md:px-6 md:pt-6">
        <h1 className={cn(TEXT_TITLE, "shrink-0")}>{dashboard.title}</h1>
        <div className="flex flex-wrap items-center gap-2">
          {offered && offered.length > 0 && range ? (
            <RangePicker
              offered={offered}
              selected={range}
              onSelect={onSelectRange}
            />
          ) : null}
          <DashboardPageActions name={name} title={dashboard.title} />
        </div>
      </header>
      {items.length === 0 ? (
        <ComingSoon
          variant="card"
          state="empty"
          label="This dashboard holds no widgets yet."
        />
      ) : (
        <div className="grid items-start gap-4 @3xl:grid-cols-2">
          {items.map((item, index) =>
            "widget" in item ? (
              <DashboardWidgetSlot
                key={`${index}-${item.widget}`}
                name={item.widget}
                range={range}
              />
            ) : "heading" in item ? (
              // The eyebrow label the portal's own sections use, and the whole
              // row: it introduces the widgets under it rather than sitting
              // beside one.
              <h2
                key={`${index}-heading`}
                className="col-span-full mt-2 text-xs font-medium tracking-wider text-muted-foreground uppercase first:mt-0"
              >
                {item.heading}
              </h2>
            ) : (
              <p
                key={`${index}-text`}
                className={cn(
                  TEXT_BODY,
                  "col-span-full max-w-prose text-muted-foreground"
                )}
              >
                {item.text}
              </p>
            )
          )}
        </div>
      )}
    </>
  );
}

function DashboardWidgetSlot({
  name,
  range,
}: {
  name: string;
  range: string | undefined;
}) {
  const [drilldown, setDrilldown] = useState(false);
  const widgetState = useQuery(widgetQuery(name));
  const metric = widgetState.data?.metric;

  // Whether a window has a date to select by decides the request, so nothing
  // runs until the definition is in: guessing shows a number the picker does
  // not claim. The metric's body cannot answer it - the date may be the
  // dataset's - so the service reports the one in force.
  const definitionState = useQuery({
    ...metricQuery(metric ?? ""),
    enabled: Boolean(metric) && Boolean(range),
  });
  const clocked = Boolean(definitionState.data?.clock);
  const known = !range || definitionState.isSuccess;
  const options: RunOptions | undefined =
    range && clocked && widgetState.data
      ? { range, bucket: drawsBucket(widgetState.data) }
      : undefined;

  const resultState = useQuery({
    ...metricResultQuery(metric ?? "", options),
    enabled: Boolean(metric) && known,
  });

  const fallbackTitle = <span className="font-mono">{name}</span>;

  if (widgetState.isPending) {
    return (
      <WidgetFrame title={fallbackTitle} state="loading">
        {null}
      </WidgetFrame>
    );
  }
  // A definition that cannot be read leaves the card unable to say whether its
  // metric is windowed, so it says that rather than running something.
  if (range && definitionState.isError) {
    return (
      <WidgetFrame
        title={fallbackTitle}
        state="error"
        errorLabel={`Couldn't read the metric behind ${name}.`}
        onRetry={() => void definitionState.refetch()}
      >
        {null}
      </WidgetFrame>
    );
  }
  if (widgetState.isError) {
    return (
      <WidgetFrame
        title={fallbackTitle}
        state="error"
        errorLabel={refusal(widgetState.error, `Couldn't read ${name}.`)}
      >
        {null}
      </WidgetFrame>
    );
  }

  const heading = widgetState.data.title;
  const label = `Show the data behind ${heading ?? name}`;

  return (
    <>
      <WidgetFrame
        title={heading ?? fallbackTitle}
        state="ready"
        scroll={widgetState.data.type === "table"}
        size={widgetState.data.type === "table" ? "tall" : "standard"}
        className={
          widgetState.data.type === "table" ? "col-span-full" : undefined
        }
        onBodyActivate={() => setDrilldown(true)}
        bodyLabel={label}
        action={
          <>
            {range && definitionState.isSuccess && !clocked ? (
              <Badge variant="secondary" className="shrink-0">
                All time
              </Badge>
            ) : null}
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={label}
              className="text-muted-foreground"
              onClick={() => setDrilldown(true)}
            >
              <Table2 />
            </Button>
          </>
        }
      >
        <CustomWidget
          widget={widgetState.data}
          result={resultState.data}
          error={resultState.error}
          windowed={Boolean(options)}
          pending={!known || resultState.isPending || resultState.isFetching}
        />
      </WidgetFrame>
      <WidgetDrilldown
        widget={widgetState.data}
        name={name}
        label={heading ?? name}
        open={drilldown}
        onOpenChange={setDrilldown}
        options={options}
      />
    </>
  );
}
