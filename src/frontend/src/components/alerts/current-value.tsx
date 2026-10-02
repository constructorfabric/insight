import { useQuery } from "@tanstack/react-query";

import type { AlertOperator } from "@/api/alerts-types";
import { refusal } from "@/components/custom/refusal";
import { Spinner } from "@/components/ui/spinner";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import { numberText, reasonText } from "@/lib/alerts/describe";
import { previewCheck } from "@/lib/alerts/preview";
import { TEXT_LABEL } from "@/lib/type-scale";
import { metricResultQuery } from "@/queries/custom";

/** Long enough that picking a metric and a window is one run, not three. */
const RUN_DEBOUNCE_MS = 600;

/**
 * What a check would read if it ran now: the value and whether it meets the
 * condition, or why the check could not tell.
 *
 * It runs the metric the way a check does, over the rule's window and as one
 * total, so a metric that answers many rows is caught before it is saved.
 */
export function CurrentValue({
  metric,
  column,
  range,
  operator,
  threshold,
}: {
  metric: string;
  column: string;
  range: string;
  operator: AlertOperator;
  threshold: number | undefined;
}) {
  const askedMetric = useDebouncedValue(metric, RUN_DEBOUNCE_MS);
  const askedRange = useDebouncedValue(range, RUN_DEBOUNCE_MS);
  const run = useQuery({
    ...metricResultQuery(askedMetric, {
      bucket: false,
      ...(askedRange ? { range: askedRange } : {}),
    }),
    // INVARIANT: a column is picked only once the metric resolved, so a half-typed name never runs.
    enabled: askedMetric !== "" && column !== "",
    // "Now" means now: the app keeps metric runs for an hour by default.
    staleTime: 0,
    retry: false,
  });

  return (
    <div aria-live="polite" aria-atomic="true" className={TEXT_LABEL}>
      {reading()}
    </div>
  );

  function reading() {
    if (metric === "" || column === "") {
      return <p>Pick a metric and a column to see what a check reads now.</p>;
    }
    if (run.isPending || askedMetric !== metric || askedRange !== range) {
      return (
        <p className="flex items-center gap-2">
          <Spinner className="size-3" /> Running the metric…
        </p>
      );
    }
    if (run.isError) {
      return (
        <p className="text-destructive">
          {refusal(run.error, "Couldn't run the metric.")}
        </p>
      );
    }

    const preview = previewCheck(run.data, column, operator, threshold ?? 0);
    if (preview.kind === "unknown") {
      return (
        <p className="text-destructive">
          Now: unknown. {reasonText(preview.reason)}
        </p>
      );
    }

    return (
      <p>
        Now: <span className="tabular-nums">{numberText(preview.value)}</span>
        {threshold === undefined
          ? ""
          : preview.breached
            ? ". It meets the condition, so a notification would be sent."
            : ". It does not meet the condition."}
      </p>
    );
  }
}
