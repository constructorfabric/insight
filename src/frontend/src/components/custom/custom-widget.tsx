import type { MetricResult, Widget } from "@/api/custom-client";
import { ChartKind } from "@/components/charts/kinds";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { refusal } from "@/components/custom/refusal";
import { CustomStat } from "@/components/custom/custom-stat";
import { CustomTable } from "@/components/custom/custom-table";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { isWidgetKind, widgetColumns } from "@/lib/custom/widget-columns";

export interface CustomWidgetProps {
  widget: Widget;
  result?: MetricResult;
  /** Whatever failed the run, shown as the service worded it. */
  error?: unknown;
  /** Whether a time range was asked for, which changes what empty means. */
  windowed?: boolean;
  /** Whether the run is still in flight, which is not the same as empty. */
  pending?: boolean;
}

export function CustomWidget({
  widget,
  result,
  error,
  windowed,
  pending,
}: CustomWidgetProps) {
  if (error) {
    return (
      <Alert variant="destructive">
        <AlertDescription>
          {refusal(error, "The metric could not be run.")}
        </AlertDescription>
      </Alert>
    );
  }

  // A run in flight has no result yet, which is not the same as a result with
  // nothing in it: saying "No data" while waiting reads as an answer.
  if (pending || !result) {
    return <CenteredSpinner className="min-h-40" />;
  }

  if (result.rows.length === 0) {
    return (
      <ComingSoon
        variant="card"
        state="empty"
        label={
          windowed ? "Nothing in this window. Try a wider range." : "No data."
        }
      />
    );
  }

  if (!isWidgetKind(widget.type)) return <p>Unknown widget type.</p>;

  const missing = drawable(widget.metric, widgetColumns(widget), result);
  if (missing) return missing;

  switch (widget.type) {
    case "table":
      return <CustomTable result={result} />;
    case "stat":
      return (
        <CustomStat result={result} value={widget.value} label={widget.label} />
      );
    default:
      return <ChartKind widget={widget} result={result} />;
  }
}

/**
 * What to render instead of the widget when its metric does not return a
 * column it draws, and nothing when it does.
 *
 * The service refuses such a widget now, but one stored before it did — or a
 * metric edited to drop a column — still reaches here, and a chart missing
 * its y column drew its axes and no line. That read as missing data rather
 * than as a definition naming a column that is not there.
 */
function drawable(
  metric: string,
  drawn: string[],
  result: MetricResult
): React.ReactElement | null {
  const missing = drawn.filter((column) => !result.columns.includes(column));
  if (missing.length === 0) return null;

  return (
    <Alert variant="destructive">
      <AlertDescription>
        This widget draws {missing.join(", ")}, which {metric} does not return.
        Its columns are {result.columns.join(", ")}.
      </AlertDescription>
    </Alert>
  );
}
