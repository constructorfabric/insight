import type { MetricResult, Widget } from "@/api/custom-client";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { refusal } from "@/components/custom/refusal";
import { CustomAreaChart } from "@/components/custom/custom-area-chart";
import { CustomBarChart } from "@/components/custom/custom-bar-chart";
import { CustomLineChart } from "@/components/custom/custom-line-chart";
import { CustomPieChart } from "@/components/custom/custom-pie-chart";
import { CustomStat } from "@/components/custom/custom-stat";
import { CustomTable } from "@/components/custom/custom-table";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";

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

  switch (widget.type) {
    case "table":
      return (
        drawable(widget.metric, widget.columns, result) ?? (
          <CustomTable result={result} />
        )
      );
    case "line":
      return (
        drawable(widget.metric, [widget.x, widget.y], result) ?? (
          <CustomLineChart result={result} x={widget.x} y={widget.y} />
        )
      );
    case "bar":
      return (
        drawable(widget.metric, [widget.x, widget.y], result) ?? (
          <CustomBarChart result={result} x={widget.x} y={widget.y} />
        )
      );
    case "area":
      return (
        drawable(widget.metric, [widget.x, widget.y], result) ?? (
          <CustomAreaChart result={result} x={widget.x} y={widget.y} />
        )
      );
    case "stat":
      return (
        drawable(widget.metric, [widget.value], result) ?? (
          <CustomStat
            result={result}
            value={widget.value}
            label={widget.label}
          />
        )
      );
    case "pie":
      return (
        drawable(widget.metric, [widget.label, widget.value], result) ?? (
          <CustomPieChart
            result={result}
            label={widget.label}
            value={widget.value}
          />
        )
      );
    default:
      return <p>Unknown widget type.</p>;
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
