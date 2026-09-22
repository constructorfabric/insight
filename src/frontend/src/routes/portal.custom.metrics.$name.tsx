import { createFileRoute, Link, useRouterState } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

import { CustomTable } from "@/components/custom/custom-table";
import { MetricSummary } from "@/components/custom/definition-summary";
import { EditLink } from "@/components/custom/editor/edit-link";
import { HeldBy } from "@/components/custom/held-by";
import { refusal } from "@/components/custom/refusal";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { metricQuery, metricResultQuery } from "@/queries/custom";
import { RANGE_PRESETS } from "@/lib/custom/time-range";
import { TEXT_BODY, TEXT_HEADING, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export const Route = createFileRoute("/portal/custom/metrics/$name")({
  component: MetricPage,
});

/** The windows a look may run over: the board's presets, without all time, which is the run with no window. */
const WINDOWS = RANGE_PRESETS.filter(({ token }) => token !== "inf");

function metricNameFromPath(pathname: string): string {
  const match = pathname.match(/^\/portal\/custom\/metrics\/([^/]+)/);

  return match ? decodeURIComponent(match[1]) : "";
}

function MetricPage() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const name = metricNameFromPath(pathname);
  const metric = useQuery(metricQuery(name));

  if (metric.isPending) return <CenteredSpinner className="min-h-40" />;

  if (metric.isError) {
    return (
      <div className="flex flex-col gap-2 p-4 md:p-6">
        <h1 className={cn(TEXT_HEADING, "font-mono")}>{name}</h1>
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {refusal(metric.error, "That metric is not there.")}
        </p>
        <Link
          to="/portal/custom/metrics"
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
      <header className="flex flex-wrap items-baseline gap-3">
        <div className="flex min-w-0 flex-col gap-1">
          <h1 className={cn(TEXT_HEADING, "font-mono")}>{name}</h1>
          <Link
            to="/portal/custom/metrics"
            className={cn(
              TEXT_BODY,
              "self-start underline decoration-dotted underline-offset-4"
            )}
          >
            Back to the catalogue
          </Link>
        </div>
        <span className="ms-auto">
          <EditLink kind="metrics" name={name} />
        </span>
      </header>

      <Card>
        <CardHeader>
          <CardTitle className={TEXT_HEADING}>Query</CardTitle>
        </CardHeader>
        <CardContent>
          <MetricSummary definition={metric.data.definition} />
          <p className={cn(TEXT_LABEL, "mt-3 text-muted-foreground")}>
            {metric.data.clock
              ? `Windowed by ${metric.data.clock.field}, the ${metric.data.clock.from === "dataset" ? "dataset's main date" : "metric's own date"}.`
              : "Nothing dates this metric: every run reads all time."}
          </p>
        </CardContent>
      </Card>

      <Run name={name} windowed={metric.data.clock !== undefined} />

      <HeldBy kind="metrics" name={name} />
    </div>
  );
}

/** The metric run here and now, over a window the reader picks. */
function Run({ name, windowed }: { name: string; windowed: boolean }) {
  const [range, setRange] = useState<string | undefined>(undefined);
  const result = useQuery(
    metricResultQuery(name, range ? { range } : undefined)
  );

  return (
    <Card>
      <CardHeader className="flex flex-row flex-wrap items-center gap-2">
        <CardTitle className={TEXT_HEADING}>Result</CardTitle>
        {windowed ? (
          <span className="ms-auto flex flex-wrap items-center gap-1">
            <Window
              label="All time"
              chosen={range === undefined}
              onPick={() => setRange(undefined)}
            />
            {WINDOWS.map(({ token, label }) => (
              <Window
                key={token}
                label={label}
                chosen={range === token}
                onPick={() => setRange(token)}
              />
            ))}
          </span>
        ) : null}
      </CardHeader>
      <CardContent>
        {result.isPending ? (
          <CenteredSpinner className="min-h-24" />
        ) : result.isError ? (
          <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
            {refusal(result.error, "The metric could not be run.")}
          </p>
        ) : result.data.rows.length === 0 ? (
          <p className={cn(TEXT_BODY, "text-muted-foreground")}>
            No rows for this window.
          </p>
        ) : (
          <>
            <p className={cn(TEXT_LABEL, "mb-2 text-muted-foreground")}>
              {result.data.rows.length} rows
            </p>
            <CustomTable result={result.data} />
          </>
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
