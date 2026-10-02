import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

import type { AlertDestination, AlertOperator } from "@/api/alerts-types";
import type { StoredMetric } from "@/api/custom-types";
import { FieldSelect } from "@/components/alerts/field-select";
import { MetricPicker } from "@/components/alerts/metric-picker";
import { Row } from "@/components/custom/editor/controls";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { OPERATORS } from "@/lib/alerts/describe";
import {
  thresholdOf,
  type AlertForm,
  type FieldErrors,
} from "@/lib/alerts/draft";
import {
  INTERVAL_PRESETS_SECS,
  fromSeconds,
  intervalLabel,
  toSeconds,
  type IntervalUnit,
} from "@/lib/alerts/interval";
import {
  alertColumns,
  isRatio,
  metricShape,
  type MetricShape,
} from "@/lib/alerts/shape";
import { windowFor, windowRule, type WindowRule } from "@/lib/alerts/window";
import { describing } from "@/lib/custom/editor/aria";
import { RANGE_PRESETS, rangeLabel } from "@/lib/custom/time-range";
import { metricQuery } from "@/queries/custom";

import { CurrentValue } from "./current-value";

const CUSTOM = "custom";

/** The window value meaning no window, since a kit select takes no empty value. */
const ALL_TIME = "all-time";

/** The service resolves every calendar window against a UTC clock. */
const UTC_NOTE = "Days, months and quarters end at midnight UTC.";

/** The windows bounded by calendar days rather than counted back from now. */
const CALENDAR_WINDOWS = new Set(["PDC", "PMC", "PQC"]);

const UNITS: readonly IntervalUnit[] = ["minutes", "hours", "days"];

/** The columns a metric answers, by the names a rule calls them. */
function useStoredMetric(metric: string): StoredMetric | undefined {
  const read = useQuery({ ...metricQuery(metric), enabled: metric !== "" });

  return read.data;
}

/** Why the picked metric cannot be alerted on, or nothing when it can. */
function shapeProblem(shape: MetricShape): string | undefined {
  switch (shape.kind) {
    case "one":
      return undefined;
    case "grouped":
      return `Returns one row per ${shape.by.join(", ")}. Alerts need a single value.`;
    case "rows":
      return "Returns multiple rows. Alerts need a single value.";
  }
}

/** The windows a rule may name: the presets, and the one it already has. */
function windowOptions(current: string, rule: WindowRule) {
  // INVARIANT: "inf" reads every dated row, which "All time" already offers.
  const presets = RANGE_PRESETS.filter(({ token }) => token !== "inf").map(
    ({ token, label }) => ({ value: token, label, disabled: !rule.windowed })
  );
  const known = current === "" || presets.some((one) => one.value === current);

  return [
    { value: ALL_TIME, label: "All time", disabled: !rule.allTime },
    ...presets,
    ...(known
      ? []
      : [
          {
            value: current,
            label: rangeLabel(current),
            disabled: !rule.windowed,
          },
        ]),
  ];
}

export function AlertFields({
  form,
  errors,
  destinations,
  onChange,
}: {
  form: AlertForm;
  errors: FieldErrors;
  destinations: readonly AlertDestination[];
  onChange: (next: AlertForm) => void;
}) {
  const stored = useStoredMetric(form.metric);
  const definition = stored?.definition;
  const window = windowRule(stored, form.column);
  const windowHint = CALENDAR_WINDOWS.has(form.range) ? UTC_NOTE : undefined;
  const metricProblem =
    errors.metric ??
    (definition ? shapeProblem(metricShape(definition)) : undefined);
  const columns = definition
    ? alertColumns(definition).map((field) => field.as_name)
    : [];
  const set = (patch: Partial<AlertForm>) => onChange({ ...form, ...patch });

  const secs = toSeconds(form.interval);
  // Custom is a choice of its own: typing an amount that happens to equal a
  // preset must not snap the control back to the preset mid-edit.
  const [custom, setCustom] = useState(!INTERVAL_PRESETS_SECS.includes(secs));
  const preset = custom ? CUSTOM : String(secs);

  return (
    <div className="flex flex-col gap-6">
      <Row id="alert-name" label="Name" required said={errors.name}>
        <Input
          id="alert-name"
          {...describing("alert-name", { said: errors.name, required: true })}
          value={form.name}
          className="h-9 w-full"
          onChange={(event) => set({ name: event.target.value })}
        />
      </Row>

      <Row id="alert-metric" label="Metric" required said={metricProblem}>
        <MetricPicker
          id="alert-metric"
          value={form.metric}
          describe={describing("alert-metric", {
            said: metricProblem,
            required: true,
          })}
          onChange={(metric) => set({ metric, column: "" })}
        />
      </Row>
      <div className="grid gap-4 sm:grid-cols-2">
        <Row id="alert-column" label="Column" required said={errors.column}>
          <FieldSelect
            id="alert-column"
            value={form.column}
            placeholder={form.metric ? "Pick a column" : "Pick a metric first"}
            disabled={form.metric === ""}
            options={[
              ...columns.map((name) => ({ value: name, label: name })),
              ...(form.column && !columns.includes(form.column)
                ? [{ value: form.column, label: form.column }]
                : []),
            ]}
            describe={describing("alert-column", {
              said: errors.column,
              required: true,
            })}
            className="font-mono"
            onChange={(column) =>
              set({ column, range: windowFor(stored, column, form.range) })
            }
          />
        </Row>
        <Row
          id="alert-range"
          label="Window"
          hint={windowHint}
          said={errors.range}
        >
          <FieldSelect
            id="alert-range"
            value={form.range === "" ? ALL_TIME : form.range}
            options={windowOptions(form.range, window)}
            disabled={!window.windowed}
            describe={describing("alert-range", {
              hint: windowHint,
              said: errors.range,
            })}
            onChange={(range) =>
              set({ range: range === ALL_TIME ? "" : range })
            }
          />
        </Row>
      </div>

      <div className="grid gap-4 sm:grid-cols-[minmax(0,12rem)_minmax(0,1fr)]">
        <Row id="alert-operator" label="Condition">
          <FieldSelect
            id="alert-operator"
            value={form.operator}
            options={OPERATORS}
            onChange={(operator) =>
              set({ operator: operator as AlertOperator })
            }
          />
        </Row>
        <Row
          id="alert-threshold"
          label="Threshold"
          required
          said={errors.threshold}
        >
          <Input
            id="alert-threshold"
            {...describing("alert-threshold", {
              said: errors.threshold,
              required: true,
            })}
            inputMode="decimal"
            value={form.threshold}
            className="h-9 w-full tabular-nums"
            onChange={(event) => set({ threshold: event.target.value })}
          />
        </Row>
      </div>
      <CurrentValue
        metric={form.metric}
        column={form.column}
        range={form.range}
        operator={form.operator}
        threshold={thresholdOf(form.threshold)}
        ratio={isRatio(definition, form.column)}
      />

      <div className="grid gap-4 sm:grid-cols-2">
        <Row
          id="alert-interval"
          label="Check every"
          said={errors.interval_secs}
        >
          <FieldSelect
            id="alert-interval"
            value={preset}
            options={[
              ...INTERVAL_PRESETS_SECS.map((one) => ({
                value: String(one),
                label: intervalLabel(one),
              })),
              { value: CUSTOM, label: "Custom…" },
            ]}
            describe={describing("alert-interval", {
              said: errors.interval_secs,
            })}
            onChange={(picked) => {
              setCustom(picked === CUSTOM);
              if (picked !== CUSTOM) {
                set({ interval: fromSeconds(Number(picked)) });
              }
            }}
          />
          {preset === CUSTOM ? (
            <span className="mt-2 flex items-center gap-2">
              <Input
                id="alert-interval-amount"
                aria-label="Amount"
                {...describing("alert-interval", {
                  said: errors.interval_secs,
                })}
                type="number"
                min={1}
                step={1}
                value={
                  Number.isNaN(form.interval.amount) ? "" : form.interval.amount
                }
                className="h-9 w-24 tabular-nums"
                onChange={(event) =>
                  set({
                    interval: {
                      ...form.interval,
                      amount: event.target.valueAsNumber,
                    },
                  })
                }
              />
              <FieldSelect
                id="alert-interval-unit"
                label="Unit"
                value={form.interval.unit}
                options={UNITS.map((unit) => ({ value: unit, label: unit }))}
                className="w-28"
                onChange={(unit) =>
                  set({
                    interval: {
                      ...form.interval,
                      unit: unit as IntervalUnit,
                    },
                  })
                }
              />
            </span>
          ) : null}
        </Row>
        <Row
          id="alert-destination"
          label="Destination"
          required
          said={errors.destination}
        >
          <FieldSelect
            id="alert-destination"
            value={form.destination}
            placeholder="Pick a destination"
            options={[
              ...destinations.map(({ name, provider }) => ({
                value: name,
                label: `${name} (${provider})`,
              })),
              ...(form.destination &&
              !destinations.some(({ name }) => name === form.destination)
                ? [{ value: form.destination, label: form.destination }]
                : []),
            ]}
            describe={describing("alert-destination", {
              said: errors.destination,
              required: true,
            })}
            onChange={(destination) => set({ destination })}
          />
        </Row>
      </div>

      <label className="flex items-center gap-3 self-start">
        <Switch
          checked={form.enabled}
          onCheckedChange={(enabled: boolean) => set({ enabled })}
        />
        <span className="text-sm">Enabled</span>
      </label>
    </div>
  );
}
