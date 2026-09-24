import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, Maximize2, Minimize2 } from "lucide-react";

import type { Widget } from "@/api/custom-client";
import { CustomTable } from "@/components/custom/custom-table";
import { refusal } from "@/components/custom/refusal";
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
import { metricQuery, metricResultQuery } from "@/queries/custom";
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
            {/* me-7 clears the kit's own close button, which is positioned
                over the corner rather than laid out in this row. */}
            <Button
              variant="ghost"
              size="icon-sm"
              className="ms-auto me-7"
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

  // The rows have to be the rows behind the number on the card, so they take
  // the card's own window — and a metric nothing dates cannot be windowed at
  // all.
  const windowed = options && Boolean(definition.data?.clock);
  const result = useQuery({
    ...metricResultQuery(metric, windowed ? options : undefined),
    enabled: definition.isSuccess,
  });

  if (definition.isError) {
    return (
      <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
        {refusal(definition.error, "That metric is not there.")}
      </p>
    );
  }
  if (definition.isPending || result.isPending)
    return <CenteredSpinner className="min-h-40" />;
  if (result.isError) {
    return (
      <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
        {refusal(result.error, "The metric could not be run.")}
      </p>
    );
  }
  if (!result.data || result.data.rows.length === 0) {
    return <p className={TEXT_BODY}>No data.</p>;
  }

  return (
    <div className="flex min-w-0 flex-col gap-2">
      <div
        className={cn(
          "min-w-0 overflow-auto",
          filling ? "max-h-none" : "max-h-[70vh]"
        )}
      >
        <CustomTable result={result.data} />
      </div>
    </div>
  );
}
