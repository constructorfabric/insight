import { createFileRoute, Link, useRouterState } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

import type { RunOptions, Widget } from "@/api/custom-client";
import { CustomWidget } from "@/components/custom/custom-widget";
import { WidgetSummary } from "@/components/custom/definition-summary";
import { EditLink } from "@/components/custom/editor/edit-link";
import { refusal } from "@/components/custom/refusal";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { drawsBucket } from "@/lib/custom/draws-bucket";
import { metricQuery, metricResultQuery, widgetQuery } from "@/queries/custom";
import { TEXT_BODY, TEXT_HEADING, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export const Route = createFileRoute("/portal/custom/widgets/$name")({
  component: WidgetPage,
});

/** The windows a card may be drawn over, as the service names them. */
const WINDOWS = ["PDC", "P7D", "P30D", "PMC", "PQC", "P1Y"] as const;

function widgetNameFromPath(pathname: string): string {
  const match = pathname.match(/^\/portal\/custom\/widgets\/([^/]+)/);

  return match ? decodeURIComponent(match[1]) : "";
}

function WidgetPage() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const name = widgetNameFromPath(pathname);
  const widget = useQuery(widgetQuery(name));

  if (widget.isPending) return <CenteredSpinner className="min-h-40" />;

  if (widget.isError) {
    return (
      <div className="flex flex-col gap-2 p-4 md:p-6">
        <h1 className={cn(TEXT_HEADING, "font-mono")}>{name}</h1>
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {refusal(widget.error, "That widget is not there.")}
        </p>
        <Link
          to="/portal/custom/widgets"
          className={cn(
            TEXT_BODY,
            "underline decoration-dotted underline-offset-4"
          )}
        >
          Back to the catalogue
        </Link>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4 p-4 md:p-6">
      <header className="flex flex-wrap items-center gap-3">
        <h1 className={cn(TEXT_HEADING, "font-mono")}>{name}</h1>
        <span className="ms-auto">
          <EditLink kind="widgets" name={name} />
        </span>
      </header>

      <Card>
        <CardHeader>
          <CardTitle className={TEXT_HEADING}>Definition</CardTitle>
        </CardHeader>
        <CardContent>
          <WidgetSummary widget={widget.data} />
        </CardContent>
      </Card>

      <Drawn name={name} widget={widget.data} />
    </div>
  );
}

/** The card as a board would draw it, over a window the reader picks. */
function Drawn({ name, widget }: { name: string; widget: Widget }) {
  const [range, setRange] = useState<string | undefined>(undefined);
  const metric = useQuery(metricQuery(widget.metric));
  const clocked = metric.data?.clock !== undefined;
  // As on a board: a window only when the metric has a date to select by,
  // sliced into buckets when the chart draws them.
  const options: RunOptions | undefined =
    range && clocked ? { range, bucket: drawsBucket(widget) } : undefined;
  const result = useQuery({
    ...metricResultQuery(widget.metric, options),
    enabled: metric.isSuccess,
  });

  return (
    <Card>
      <CardHeader className="flex flex-row flex-wrap items-center gap-2">
        <CardTitle className={TEXT_HEADING}>{widget.title ?? name}</CardTitle>
        <span className="ms-auto flex flex-wrap items-center gap-1">
          <Window
            label="All time"
            chosen={range === undefined}
            onPick={() => setRange(undefined)}
          />
          {WINDOWS.map((token) => (
            <Window
              key={token}
              label={token}
              chosen={range === token}
              onPick={() => setRange(token)}
            />
          ))}
        </span>
      </CardHeader>
      <CardContent>
        {range && metric.isSuccess && !clocked ? (
          <p className={cn(TEXT_LABEL, "mb-2 text-muted-foreground")}>
            Nothing dates this widget's metric, so every window shows all time.
          </p>
        ) : null}
        {metric.isPending || result.isPending ? (
          <CenteredSpinner className="min-h-40" />
        ) : (
          <CustomWidget
            widget={widget}
            result={result.data}
            error={
              result.error
                ? new Error(
                    refusal(result.error, "The metric could not be run.")
                  )
                : metric.error
                  ? new Error(
                      refusal(metric.error, "The metric could not be read.")
                    )
                  : undefined
            }
            windowed={Boolean(options)}
          />
        )}
      </CardContent>
    </Card>
  );
}

function Window({
  label,
  chosen,
  onPick,
}: {
  label: string;
  chosen: boolean;
  onPick: () => void;
}) {
  return (
    <Button
      variant={chosen ? "secondary" : "ghost"}
      size="sm"
      aria-pressed={chosen}
      onClick={onPick}
    >
      {label}
    </Button>
  );
}
