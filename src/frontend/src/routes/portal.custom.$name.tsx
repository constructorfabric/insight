import { createFileRoute, useRouterState } from "@tanstack/react-router";
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Ellipsis, Maximize2, Table2 } from "lucide-react";

import { Button, Badge } from "@gears-frontx/ui-kit";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

import {
  CustomApiError,
  isBusy,
  type DashboardRead,
  type DashboardWidget,
  type RunOptions,
} from "@/api/custom-client";
import { DashboardByline } from "@/components/custom/dashboard-byline";
import { DashboardPageActions } from "@/components/custom/dashboard-page-actions";
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
import { ExpandedWidget } from "@/components/custom/expanded-widget";
import { refusal } from "@/components/custom/refusal";
import { WidgetDrilldown } from "@/components/custom/widget-drilldown";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import {
  dashboardReadQuery,
  dashboardWidgetResultQuery,
  dashboardWidgetsQuery,
} from "@/queries/custom";
import { useIsAdmin } from "@/queries/identity-me";
import { TEXT_BODY, TEXT_HEADING, TEXT_TITLE } from "@/lib/type-scale";
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
  if (isLoading)
    return (
      <CenteredSpinner
        data-testid="page-custom-dashboard"
        className="min-h-40"
      />
    );
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

  const picksRange = offered !== undefined && offered.length > 0 && range;

  return (
    <>
      {picksRange ? (
        <div
          role="toolbar"
          aria-label="Period"
          className="sticky -top-4 z-10 -mx-4 -mt-4 mb-4 flex min-h-[53px] items-center justify-end border-b bg-background px-4 py-2 md:-top-6 md:-mx-6 md:-mt-6 md:px-6"
        >
          <RangePicker
            offered={offered}
            selected={range}
            onSelect={onSelectRange}
          />
        </div>
      ) : null}
      <header
        data-testid="page-custom-dashboard"
        className="mb-6 flex flex-col gap-1.5"
      >
        <div className="flex items-start justify-between gap-3">
          <h1 className={cn(TEXT_TITLE, "min-w-0 break-words")}>
            {dashboard.title || read.name}
          </h1>
          <DashboardPageActions
            id={id}
            name={read.name}
            title={dashboard.title}
            access={read.access}
            ownerPersonId={read.ownerPersonId}
            carries={read.carries}
          />
        </div>
        <DashboardByline
          owner={read.ownerName}
          updatedAt={read.updatedAt}
          visibility={read.visibility}
          access={read.access}
          folders={read.folders}
        />
      </header>
      {items.length === 0 ? (
        <ComingSoon
          variant="card"
          state="empty"
          label="This dashboard holds no widgets yet."
        />
      ) : (
        <DashboardItems dashboard={id} items={items} range={range} />
      )}
    </>
  );
}

function DashboardItems({
  dashboard,
  items,
  range,
}: {
  dashboard: string;
  items: ReturnType<typeof dashboardItems>;
  range: string | undefined;
}) {
  const search = usePortalSearch();
  const setSearch = useSetPortalSearch();
  const widgets = useQuery(dashboardWidgetsQuery(dashboard));
  const byId = new Map(widgets.data?.map((widget) => [widget.id, widget]));
  const named = search.widget ? byId.get(search.widget) : undefined;
  const expanded = named && expands(named) ? named : undefined;

  if (widgets.isError) {
    return (
      <ComingSoon
        variant="card"
        state="error"
        label={refusal(widgets.error, "Couldn't load the widgets.")}
        onRetry={() => void widgets.refetch()}
      />
    );
  }

  return (
    <>
      <div className="grid items-start gap-4 @3xl:grid-cols-2">
        {items.map((item, index) =>
          "widget" in item ? (
            <DashboardWidgetSlot
              key={`${index}-${item.widget}`}
              dashboard={dashboard}
              id={item.widget}
              widget={byId.get(item.widget)}
              pending={widgets.isPending}
              range={range}
              onExpand={() => setSearch({ widget: item.widget })}
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
      {expanded ? (
        <ExpandedWidget
          dashboard={dashboard}
          widget={expanded}
          heading={widgetHeading(expanded)}
          options={runOptions(expanded, range)}
          allTime={drawnOverAllTime(expanded, range)}
          onClose={() => setSearch({ widget: undefined }, { replace: true })}
        />
      ) : null}
    </>
  );
}

// Whether a window has a date to select by decides the request. The metric's
// body cannot answer it - the date may be the dataset's - so the service
// reports the one in force with the widget.
function runOptions(
  widget: DashboardWidget,
  range: string | undefined
): RunOptions | undefined {
  return range && widget.clock
    ? { range, bucket: drawsBucket(widget.body) }
    : undefined;
}

function drawnOverAllTime(
  widget: DashboardWidget,
  range: string | undefined
): boolean {
  return Boolean(range) && !widget.clock;
}

// A stat is one number: drawn bigger, it says nothing more.
function expands(widget: DashboardWidget): boolean {
  return widget.body.type !== "stat";
}

function widgetHeading(widget: DashboardWidget): string {
  return widget.body.title ?? widget.name;
}

function DashboardWidgetSlot({
  dashboard,
  id,
  widget,
  pending,
  range,
  onExpand,
}: {
  dashboard: string;
  id: string;
  /** Absent once the widgets are read means the dashboard no longer draws it. */
  widget: DashboardWidget | undefined;
  pending: boolean;
  range: string | undefined;
  onExpand: () => void;
}) {
  const { isAdmin } = useIsAdmin();
  const [drilldown, setDrilldown] = useState(false);
  const name = widget?.name ?? id;

  const options = widget ? runOptions(widget, range) : undefined;

  const resultState = useQuery({
    ...dashboardWidgetResultQuery(dashboard, id, options),
    enabled: widget !== undefined,
  });

  if (pending) {
    return (
      <Card>
        <CardContent>
          <CenteredSpinner className="min-h-40" />
        </CardContent>
      </Card>
    );
  }
  if (!widget) {
    return (
      <Card>
        <CardContent>
          <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
            This widget is no longer on the dashboard.
          </p>
        </CardContent>
      </Card>
    );
  }

  const heading = widgetHeading(widget);
  const drawn = (
    <CustomWidget
      widget={widget.body}
      result={resultState.data}
      error={resultState.error}
      windowed={Boolean(options)}
      pending={resultState.isPending || resultState.isFetching}
      onRetry={() => void resultState.refetch()}
      onPointClick={isAdmin ? () => setDrilldown(true) : undefined}
    />
  );

  return (
    <Card>
      <CardHeader className="flex flex-row items-start gap-2">
        <CardTitle className={cn(TEXT_HEADING, "min-w-0 flex-1")}>
          {heading}
        </CardTitle>
        {drawnOverAllTime(widget, range) ? (
          <Badge variant="secondary" className="shrink-0">
            All time
          </Badge>
        ) : null}
        {expands(widget) ? (
          <Button
            variant="ghost"
            size="sm"
            aria-label={`Expand ${heading}`}
            className="-my-1 text-muted-foreground"
            icon={<Maximize2 />}
            onClick={onExpand}
          />
        ) : null}
        {isAdmin ? (
          <DropdownMenu>
            <DropdownMenuTrigger
              render={
                <Button
                  variant="ghost"
                  size="sm"
                  aria-label={`More for ${heading}`}
                  className="-my-1 text-muted-foreground"
                  icon={<Ellipsis />}
                />
              }
            />
            <DropdownMenuContent align="end">
              <DropdownMenuItem
                disabled={isBusy(resultState.error)}
                onClick={() => setDrilldown(true)}
              >
                <Table2 />
                Show data
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        ) : null}
      </CardHeader>
      <CardContent className="max-h-72 overflow-auto">{drawn}</CardContent>
      {isAdmin ? (
        <WidgetDrilldown
          dashboard={dashboard}
          widgetId={id}
          widget={widget.body}
          name={name}
          label={heading}
          open={drilldown}
          onOpenChange={setDrilldown}
          options={options}
        />
      ) : null}
    </Card>
  );
}
