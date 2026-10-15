import { useEffect } from "react";

import type { AlertDestination } from "@/api/alerts-types";
import { ConditionBuilder } from "@/components/alerts/condition-builder";
import { FieldSelect } from "@/components/alerts/field-select";
import { MetricPicker } from "@/components/alerts/metric-picker";
import { Row } from "@/components/custom/editor/controls";
import { refusal } from "@/components/custom/refusal";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { conditionColumns } from "@/lib/alerts/condition";
import {
  textOf,
  type AlertDocument,
  type FieldErrors,
} from "@/lib/alerts/draft";
import { INTERVAL_PRESETS_SECS, intervalLabel } from "@/lib/alerts/interval";
import { isRatio } from "@/lib/alerts/shape";
import { takesWindow } from "@/lib/alerts/window";
import { describing } from "@/lib/custom/editor/aria";
import { RANGE_PRESETS, rangeLabel } from "@/lib/custom/time-range";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

import { CurrentValue } from "./current-value";
import type { AlertMetric } from "./use-alert-metric";
import type { AlertPreviewState } from "./use-alert-preview";

/** The window value meaning no window, since a kit select takes no empty value. */
const ALL_TIME = "all-time";

/** The service resolves every calendar window against a UTC clock. */
const UTC_NOTE = "Days, months and quarters end at midnight UTC.";

/** The windows bounded by calendar days rather than counted back from now. */
const CALENDAR_WINDOWS = new Set(["PDC", "PMC", "PQC"]);

/** The windows a rule may name: the presets, and the one it already has. */
function windowOptions(current: string) {
  // INVARIANT: "inf" reads every dated row, which "All time" already offers.
  const presets = RANGE_PRESETS.filter(({ token }) => token !== "inf").map(
    ({ token, label }) => ({ value: token, label })
  );
  const known = current === "" || presets.some((one) => one.value === current);

  return [
    { value: ALL_TIME, label: "All time" },
    ...presets,
    ...(known ? [] : [{ value: current, label: rangeLabel(current) }]),
  ];
}

export function AlertFields({
  document,
  errors,
  destinations,
  metric: picked,
  preview,
  onChange,
}: {
  document: AlertDocument;
  errors: FieldErrors;
  destinations: readonly AlertDestination[];
  metric: AlertMetric;
  preview: AlertPreviewState;
  onChange: (patch: AlertDocument) => void;
}) {
  const { read, stored, definition, kinds, manyRows } = picked;
  const windowed = takesWindow(stored);
  const metric = textOf(document, "metric");
  const range = textOf(document, "range");
  const interval =
    typeof document.interval_secs === "number"
      ? document.interval_secs
      : undefined;
  const destination = textOf(document, "destination");
  const tested = conditionColumns(document.condition);

  // INVARIANT: a metric that lost its date refuses the window its alert still
  // holds, and the hidden select cannot clear it.
  useEffect(() => {
    if (!windowed && range !== "") onChange({ range: null });
  }, [windowed, range, onChange]);

  const windowHint = CALENDAR_WINDOWS.has(range) ? UTC_NOTE : undefined;
  const metricProblem =
    errors.metric ??
    (read.isError
      ? refusal(read.error, "Couldn't read the metric.")
      : undefined);
  const columns = definition?.fields.map((field) => field.as_name) ?? [];
  const chosen = metric !== "" && picked.settled;
  // INVARIANT: a metric that cannot be read never hides the window a rule holds.
  const showWindow = stored
    ? Boolean(stored.clock)
    : read.isError && range !== "";

  const intervals =
    interval === undefined || INTERVAL_PRESETS_SECS.includes(interval)
      ? INTERVAL_PRESETS_SECS
      : [...INTERVAL_PRESETS_SECS, interval].sort((a, b) => a - b);

  return (
    <div className="flex flex-col gap-6">
      <Row id="alert-name" label="Name" required said={errors.name}>
        <Input
          id="alert-name"
          {...describing("alert-name", { said: errors.name, required: true })}
          value={textOf(document, "name")}
          className="h-9 w-full"
          onChange={(event) => onChange({ name: event.target.value })}
        />
      </Row>

      <div className="grid gap-4 sm:grid-cols-2">
        <Row id="alert-metric" label="Metric" required said={metricProblem}>
          <div className="flex flex-col items-start gap-1">
            <MetricPicker
              id="alert-metric"
              value={metric}
              describe={describing("alert-metric", {
                said: metricProblem,
                required: true,
              })}
              onChange={(next) => onChange({ metric: next })}
            />
            {read.isError ? (
              <Button
                type="button"
                variant="link"
                className="h-auto px-0 py-1"
                onClick={() => void read.refetch()}
              >
                Retry
              </Button>
            ) : null}
          </div>
        </Row>
        {showWindow ? (
          <Row
            id="alert-range"
            label="Window"
            hint={windowHint}
            said={errors.range}
          >
            <FieldSelect
              id="alert-range"
              value={range === "" ? ALL_TIME : range}
              options={windowOptions(range)}
              describe={describing("alert-range", {
                hint: windowHint,
                said: errors.range,
              })}
              onChange={(next) =>
                onChange({ range: next === ALL_TIME ? null : next })
              }
            />
          </Row>
        ) : null}
      </div>

      {chosen ? (
        <fieldset className="flex min-w-0 flex-col gap-3">
          <legend className={cn(TEXT_LABEL, "mb-1 font-medium")}>
            Condition
          </legend>
          <ConditionBuilder
            condition={document.condition}
            columns={columns}
            kinds={kinds}
            manyRows={manyRows}
            errors={errors}
            onChange={(condition) => onChange({ condition })}
          />
          <CurrentValue
            preview={preview}
            ratioOnly={
              tested.length > 0 &&
              tested.every((column) => isRatio(definition, column))
            }
          />
        </fieldset>
      ) : null}

      <div className="grid gap-4 sm:grid-cols-2">
        <Row
          id="alert-interval"
          label="Check every"
          said={errors.interval_secs}
        >
          <FieldSelect
            id="alert-interval"
            value={interval === undefined ? "" : String(interval)}
            options={intervals.map((secs) => ({
              value: String(secs),
              label: intervalLabel(secs),
            }))}
            describe={describing("alert-interval", {
              said: errors.interval_secs,
            })}
            onChange={(next) => onChange({ interval_secs: Number(next) })}
          />
        </Row>
        <Row
          id="alert-destination"
          label="Destination"
          required
          said={errors.destination}
        >
          <FieldSelect
            id="alert-destination"
            value={destination}
            placeholder="Pick a destination"
            options={[
              ...destinations.map(({ name, provider }) => ({
                value: name,
                label: `${name} (${provider})`,
              })),
              ...(destination &&
              !destinations.some(({ name }) => name === destination)
                ? [{ value: destination, label: destination }]
                : []),
            ]}
            describe={describing("alert-destination", {
              said: errors.destination,
              required: true,
            })}
            onChange={(next) => onChange({ destination: next })}
          />
        </Row>
      </div>

      <label className="flex items-center gap-3 self-start">
        <Switch
          checked={document.enabled === true}
          onCheckedChange={(enabled: boolean) => onChange({ enabled })}
        />
        <span className="text-sm">Enabled</span>
      </label>
    </div>
  );
}
