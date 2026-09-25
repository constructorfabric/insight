import { useMemo, useState } from "react";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { ArrowLeft, Maximize2, Minimize2 } from "lucide-react";

import type { DrilldownSort, Widget } from "@/api/custom-client";
import { refusal } from "@/components/custom/refusal";
import { MetricEvidenceTable } from "@/components/metric-evidence-table";
import {
  MetricSummary,
  WidgetSummary,
} from "@/components/custom/definition-summary";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { drawable, startsOver } from "@/lib/custom/drilldown";
import { nextSort } from "@/lib/metrics/evidence-rows";
import { drilldownPagesQuery, metricQuery } from "@/queries/custom";
import type { RunOptions } from "@/api/custom-client";
import { TEXT_BODY, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/** Which of the dialog's three views is showing. */
type View =
  { kind: "rows" } | { kind: "widget" } | { kind: "metric"; name: string };

/**
 * The rows behind a widget, and the definitions that produced them.
 *
 * A chart is a shape; the question it prompts is "which rows are those?", and
 * the one after it is "what exactly is being counted?". Both are answered
 * here rather than on another page: the dialog steps to a definition and back
 * without leaving the dashboard.
 */
export function WidgetDrilldown({
  widget,
  name,
  label,
  open,
  onOpenChange,
  options,
}: {
  widget: Widget;
  name: string;
  label: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The window the card was read over, if it was read over one. */
  options?: RunOptions;
}) {
  const [view, setView] = useState<View>({ kind: "rows" });
  /** Whether the dialog has been asked to fill the window. */
  const [filling, setFilling] = useState(false);

  /** Closing returns to the rows, so the next visit starts where it should. */
  function change(next: boolean) {
    if (!next) {
      setView({ kind: "rows" });
      setFilling(false);
    }
    onOpenChange(next);
  }

  const heading =
    view.kind === "rows" ? label : view.kind === "widget" ? name : view.name;

  return (
    <Dialog open={open} onOpenChange={change}>
      <DialogContent
        className={cn(
          filling
            ? "h-[96vh] w-[98vw] max-w-[98vw] content-start"
            : "w-[min(96vw,80rem)] max-w-[min(96vw,80rem)]"
        )}
      >
        <DialogHeader>
          <DialogTitle className="flex min-w-0 items-center gap-2">
            {view.kind === "rows" ? null : (
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Back to the rows"
                onClick={() => setView({ kind: "rows" })}
              >
                <ArrowLeft />
              </Button>
            )}
            <span
              className={cn(
                "min-w-0 truncate",
                view.kind === "rows" ? "" : "font-mono"
              )}
            >
              {heading}
            </span>
            {/* Placed as the kit places its close button - over the corner,
                not in this row - so the two sit on one line, one step apart. */}
            <Button
              variant="ghost"
              size="icon"
              className="absolute top-4 right-14 z-[2] w-9"
              aria-pressed={filling}
              aria-label={
                filling ? "Shrink the dialog back" : "Fill the window"
              }
              onClick={() => setFilling((was) => !was)}
            >
              {filling ? <Minimize2 /> : <Maximize2 />}
            </Button>
          </DialogTitle>
          {view.kind === "rows" ? null : (
            <DialogDescription>
              {view.kind === "widget"
                ? "The widget, as it is stored."
                : "The metric, as it is stored."}
            </DialogDescription>
          )}
        </DialogHeader>

        {open && view.kind === "rows" ? (
          <>
            <Rows
              metric={widget.detail ?? widget.metric}
              options={options}
              filling={filling}
            />
            <Definitions widget={widget} name={name} onOpen={setView} />
          </>
        ) : null}
        {view.kind === "widget" ? (
          <WidgetSummary widget={widget} linkMetric={false} />
        ) : null}
        {view.kind === "metric" ? <StoredMetric name={view.name} /> : null}
      </DialogContent>
    </Dialog>
  );
}

/**
 * What drew this, by name.
 *
 * The identifiers are off the card because a heading is not a name — this is
 * where a reader asks for them, and each one opens its definition in place.
 */
function Definitions({
  widget,
  name,
  onOpen,
}: {
  widget: Widget;
  name: string;
  onOpen: (view: View) => void;
}) {
  const metrics = [widget.metric, widget.detail].filter(
    (metric, index, all): metric is string =>
      Boolean(metric) && all.indexOf(metric) === index
  );

  return (
    <div
      className={cn(TEXT_LABEL, "flex flex-wrap items-center gap-x-4 gap-y-1")}
    >
      <span className="flex items-center gap-1">
        Widget
        <DefinitionButton onClick={() => onOpen({ kind: "widget" })}>
          {name}
        </DefinitionButton>
      </span>
      <span className="flex items-center gap-1">
        {metrics.length > 1 ? "Metrics" : "Metric"}
        {metrics.map((metric) => (
          <DefinitionButton
            key={metric}
            onClick={() => onOpen({ kind: "metric", name: metric })}
          >
            {metric}
          </DefinitionButton>
        ))}
      </span>
    </div>
  );
}

function DefinitionButton({
  children,
  onClick,
}: {
  children: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="font-mono underline decoration-dotted underline-offset-4"
    >
      {children}
    </button>
  );
}

function StoredMetric({ name }: { name: string }) {
  const definition = useQuery(metricQuery(name));

  if (definition.isPending) return <CenteredSpinner className="min-h-40" />;
  if (definition.isError || !definition.data) {
    return (
      <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
        {refusal(definition.error, "That metric is not there.")}
      </p>
    );
  }

  return <MetricSummary definition={definition.data.definition} />;
}

/** Past this many pages the reader is asked to narrow rather than scroll. */
const MAX_PAGES = 50;

function Rows({
  metric,
  options,
  filling,
}: {
  metric: string;
  options?: RunOptions;
  /** Filling the window, the dialog scrolls rather than the rows inside it. */
  filling: boolean;
}) {
  const definition = useQuery(metricQuery(metric));
  const [sort, setSort] = useState<DrilldownSort | null>(null);

  // The rows have to be the rows behind the number on the card, so they take
  // the card's own window — and a metric nothing dates cannot be windowed at
  // all.
  const windowed = options && Boolean(definition.data?.clock);
  const pages = useInfiniteQuery({
    ...drilldownPagesQuery(metric, {
      ...(windowed ? options : {}),
      ...(sort ? { sort } : {}),
    }),
    enabled: definition.isSuccess,
  });

  const columns = useMemo(() => pages.data?.pages[0]?.columns ?? [], [pages.data]);
  const rows = useMemo(
    () =>
      pages.data?.pages.flatMap((page) =>
        page.rows.map((row) => drawable(row, columns))
      ) ?? [],
    [pages.data, columns]
  );
  // What the headers announce: the order of the rows ON SCREEN, read off the
  // page that produced them - never the order just asked for, whose rows are
  // still on their way.
  const shownSort = pages.data?.pages[0]?.selection.sort ?? null;
  const pageLimitReached =
    (pages.data?.pages.length ?? 0) >= MAX_PAGES && pages.hasNextPage;

  if (definition.isError) {
    return (
      <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
        {refusal(definition.error, "That metric is not there.")}
      </p>
    );
  }
  if (definition.isPending || pages.isPending)
    return <CenteredSpinner className="min-h-40" />;
  if (pages.isError && !pages.data) {
    return (
      <div className="flex flex-col items-start gap-2">
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {refusal(pages.error, "The metric could not be run.")}
        </p>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => void pages.refetch()}
        >
          Retry
        </Button>
      </div>
    );
  }
  if (rows.length === 0 && !pages.isFetching) {
    return <p className={TEXT_BODY}>No data.</p>;
  }

  return (
    <div
      className={cn(
        "relative flex min-w-0 flex-col",
        filling ? "h-[calc(96vh-11rem)]" : "h-[60vh]"
      )}
    >
      <MetricEvidenceTable
        metricKey={null}
        rows={rows}
        columns={columns}
        sort={shownSort}
        // The next order is counted from the one ON SCREEN, so the first
        // click on the column the service chose itself flips it rather than
        // asking for what is already there.
        onSortChange={(key) => setSort(nextSort(shownSort, key))}
        // A table made again mid-walk invalidates every cursor: the walk
        // starts over rather than retrying a page that cannot come.
        fetchNextPage={() =>
          startsOver(pages.error) ? pages.refetch() : pages.fetchNextPage()
        }
        hasNextPage={pages.hasNextPage && !pageLimitReached}
        isFetchingNextPage={pages.isFetchingNextPage}
        reordering={pages.isFetching && !pages.isFetchingNextPage}
        nextPageError={pages.isFetchNextPageError}
        pageLimitReached={pageLimitReached}
      />
    </div>
  );
}
