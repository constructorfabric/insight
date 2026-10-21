import { createFileRoute, useRouterState } from "@tanstack/react-router";
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Table2 } from "lucide-react";

import { Button } from "@/components/ui/button";

import {
  CustomApiError,
  type DashboardRead,
  type RunOptions,
} from "@/api/custom-client";
import { DashboardPageActions } from "@/components/custom/dashboard-page-actions";
import { PersonName } from "@/components/custom/person-name";
import { RangePicker } from "@/components/custom/range-picker";
import { selectedRange } from "@/lib/custom/board-range";
import { dashboardIdFromPath } from "@/lib/custom/dashboard-path";
import { drawsBucket } from "@/lib/custom/draws-bucket";
import {
  useSetPortalSearch,
  usePortalSearch,
} from "@/lib/portal/portal-search";
import { dashboardItems } from "@/lib/custom/dashboard-items";
import { CustomWidget } from "@/components/custom/custom-widget";
import { refusal } from "@/components/custom/refusal";
import { WidgetDrilldown } from "@/components/custom/widget-drilldown";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import {
  dashboardReadQuery,
  metricQuery,
  metricResultQuery,
  widgetQuery,
} from "@/queries/custom";
import {
  TEXT_BODY,
  TEXT_HEADING,
  TEXT_LABEL,
  TEXT_TITLE,
} from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export const Route = createFileRoute("/portal/custom/$name")({
  component: CustomDashboardPage,
});

function CustomDashboardPage() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const id = dashboardIdFromPath(pathname);
  const search = usePortalSearch();
  const setSearch = useSetPortalSearch();
  const {
    data: read,
    isLoading,
    isError,
    error,
    refetch,
  } = useQuery(dashboardReadQuery(id));

  const offered = read?.body.time_ranges;
  const range = selectedRange(offered, read?.body.default_range, search.range);
  return (
    <CustomDashboardBody
      read={read}
      isLoading={isLoading}
      isError={isError}
      error={error}
      id={id}
      onRetry={() => void refetch()}
      range={range}
      offered={offered}
      onSelectRange={(token) => setSearch({ range: token })}
    />
  );
}

function CustomDashboardBody({
  read,
  isLoading,
  isError,
  error,
  id,
  onRetry,
  range,
  offered,
  onSelectRange,
}: {
  read: DashboardRead | undefined;
  isLoading: boolean;
  isError: boolean;
  error: Error | null;
  id: string;
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
          label="That dashboard is not there."
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
  if (!read) return null;

  const dashboard = read.body;
  const items = dashboardItems(dashboard);

  return (
    <>
      <header className="sticky -top-4 z-10 -mx-4 -mt-4 mb-4 flex flex-wrap items-center justify-between gap-x-3 gap-y-2 border-b bg-background px-4 pt-4 pb-3 md:-top-6 md:-mx-6 md:-mt-6 md:px-6 md:pt-6">
        <div className="flex min-w-0 flex-col">
          <h1 className={cn(TEXT_TITLE, "shrink-0")}>
            {dashboard.title || read.name}
          </h1>
          {read.ownerPersonId ? (
            <p className={cn(TEXT_LABEL, "text-muted-foreground")}>
              Owner: <PersonName personId={read.ownerPersonId} />
            </p>
          ) : null}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {offered && offered.length > 0 && range ? (
            <RangePicker
              offered={offered}
              selected={range}
              onSelect={onSelectRange}
            />
          ) : null}
          <DashboardPageActions
            id={id}
            name={read.name}
            title={dashboard.title}
            access={read.access}
            ownerPersonId={read.ownerPersonId}
          />
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
                id={item.widget}
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
  id,
  range,
}: {
  id: string;
  range: string | undefined;
}) {
  const [drilldown, setDrilldown] = useState(false);
  const widgetState = useQuery(widgetQuery(id));
  const widget = widgetState.data?.widget;
  const name = widgetState.data?.name ?? id;
  const metric = widget?.metric;

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
    range && clocked && widget
      ? { range, bucket: drawsBucket(widget) }
      : undefined;

  const resultState = useQuery({
    ...metricResultQuery(metric ?? "", options),
    enabled: Boolean(metric) && known,
  });

  if (widgetState.isPending) {
    return (
      <Card>
        <CardContent>
          <CenteredSpinner className="min-h-40" />
        </CardContent>
      </Card>
    );
  }
  // A definition that cannot be read leaves the card unable to say whether its
  // metric is windowed, so it says that rather than running something.
  if (range && definitionState.isError) {
    return (
      <Card>
        <CardContent>
          <ComingSoon
            variant="card"
            state="error"
            label={`Couldn't read the metric behind ${name}.`}
            onRetry={() => void definitionState.refetch()}
          />
        </CardContent>
      </Card>
    );
  }
  if (widgetState.isError) {
    return (
      <Card>
        <CardContent>
          <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
            {refusal(widgetState.error, `Couldn't read ${name}.`)}
          </p>
        </CardContent>
      </Card>
    );
  }

  const heading = widgetState.data.widget.title;

  return (
    <Card>
      <CardHeader className="flex flex-row items-start gap-2">
        <CardTitle className={cn(TEXT_HEADING, "min-w-0 flex-1")}>
          {heading ?? name}
        </CardTitle>
        {range && definitionState.isSuccess && !clocked ? (
          <Badge variant="secondary" className="shrink-0">
            All time
          </Badge>
        ) : null}
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={`Show the data behind ${heading ?? name}`}
          className="text-muted-foreground"
          onClick={() => setDrilldown(true)}
        >
          <Table2 />
        </Button>
      </CardHeader>
      <CardContent
        role="button"
        tabIndex={0}
        aria-label={`Show the data behind ${heading ?? name}`}
        className="max-h-72 cursor-pointer overflow-auto"
        onClick={() => setDrilldown(true)}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            setDrilldown(true);
          }
        }}
      >
        <CustomWidget
          widget={widgetState.data.widget}
          result={resultState.data}
          error={resultState.error}
          windowed={Boolean(options)}
          pending={!known || resultState.isPending || resultState.isFetching}
        />
      </CardContent>
      <WidgetDrilldown
        widget={widgetState.data.widget}
        name={name}
        label={heading ?? name}
        open={drilldown}
        onOpenChange={setDrilldown}
        options={options}
      />
    </Card>
  );
}
