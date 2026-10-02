import { useQuery } from "@tanstack/react-query";
import { useState, type ReactNode } from "react";

import type { AlertDestination, AlertOperator } from "@/api/alerts-types";
import { ReferenceControl, Row } from "@/components/custom/editor/controls";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import { OPERATORS } from "@/lib/alerts/describe";
import {
  thresholdOf,
  type AlertForm,
  type FieldErrors,
} from "@/lib/alerts/draft";
import {
  INTERVAL_PRESETS_SECS,
  fromSeconds,
  intervalText,
  toSeconds,
  type IntervalUnit,
} from "@/lib/alerts/interval";
import { describing } from "@/lib/custom/editor/aria";
import { RANGE_PRESETS, rangeLabel } from "@/lib/custom/time-range";
import { TEXT_HEADING, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import { catalogueNamesQuery, metricQuery } from "@/queries/custom";

import { CurrentValue } from "./current-value";

const SELECT =
  "h-9 w-full rounded-md border border-input bg-transparent px-3 text-sm";

const CUSTOM = "custom";

const UNITS: readonly IntervalUnit[] = ["minutes", "hours", "days"];

const METRIC_HINT = "A stored metric. The check reads one row of it.";

// Native rather than the kit's Select, as the definition editor does: a test
// drives it with `selectOptions`, and the platform owns its keyboard handling.
function Choice({
  id,
  label,
  value,
  options,
  describe,
  onChange,
  className,
}: {
  id: string;
  /** Names the control when no visible label does. */
  label?: string;
  value: string;
  options: readonly { value: string; label: string }[];
  describe?: ReturnType<typeof describing>;
  onChange: (value: string) => void;
  className?: string;
}) {
  return (
    <select
      id={id}
      aria-label={label}
      {...describe}
      value={value}
      className={cn(SELECT, className)}
      onChange={(event) => onChange(event.target.value)}
    >
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <fieldset className="flex flex-col gap-4">
      <legend className={cn(TEXT_HEADING, "mb-1")}>{title}</legend>
      {children}
    </fieldset>
  );
}

/** The columns a metric answers, by the names a rule calls them. */
/** Long enough that a metric's name is typed before it is looked up. */
const LOOKUP_DEBOUNCE_MS = 400;

function useMetricColumns(metric: string): string[] {
  const settled = useDebouncedValue(metric, LOOKUP_DEBOUNCE_MS);
  // INVARIANT: the catalogue list stops at 200 names, so it cannot gate this lookup; a pause in typing does.
  const read = useQuery({
    ...metricQuery(settled),
    enabled: settled !== "" && settled === metric,
    retry: false,
  });

  return read.data?.definition.fields.map((field) => field.as_name) ?? [];
}

/** The windows a rule may name: the presets, and the one it already has. */
function windowOptions(current: string) {
  // INVARIANT: "inf" reads every dated row, which "Every row" already offers.
  const presets = RANGE_PRESETS.filter(({ token }) => token !== "inf").map(
    ({ token, label }) => ({ value: token, label })
  );
  const known = current === "" || presets.some((one) => one.value === current);

  return [
    { value: "", label: "Every row" },
    ...presets,
    ...(known ? [] : [{ value: current, label: rangeLabel(current) }]),
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
  const metrics = useQuery(catalogueNamesQuery("metrics"));
  const columns = useMetricColumns(form.metric);
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

      <Section title="What to watch">
        <Row
          id="alert-metric"
          label="Metric"
          required
          hint={METRIC_HINT}
          said={errors.metric}
        >
          <ReferenceControl
            id="alert-metric"
            value={form.metric}
            names={metrics.data ?? []}
            describe={describing("alert-metric", {
              hint: METRIC_HINT,
              said: errors.metric,
              required: true,
            })}
            onChange={(metric) => set({ metric, column: "" })}
          />
        </Row>
        <div className="grid gap-4 sm:grid-cols-2">
          <Row id="alert-column" label="Column" required said={errors.column}>
            <Choice
              id="alert-column"
              value={form.column}
              options={[
                {
                  value: "",
                  label: form.metric ? "Pick a column" : "Pick a metric first",
                },
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
              onChange={(column) => set({ column })}
            />
          </Row>
          <Row id="alert-range" label="Window" said={errors.range}>
            <Choice
              id="alert-range"
              value={form.range}
              options={windowOptions(form.range)}
              describe={describing("alert-range", { said: errors.range })}
              onChange={(range) => set({ range })}
            />
          </Row>
        </div>
      </Section>

      <Section title="When to notify">
        <div className="grid gap-4 sm:grid-cols-[minmax(0,12rem)_minmax(0,1fr)]">
          <Row id="alert-operator" label="The value is">
            <Choice
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
        />

        <div className="grid gap-4 sm:grid-cols-2">
          <Row id="alert-interval" label="Check" said={errors.interval_secs}>
            <Choice
              id="alert-interval"
              value={preset}
              options={[
                ...INTERVAL_PRESETS_SECS.map((one) => ({
                  value: String(one),
                  label: intervalText(one),
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
                <span className={TEXT_LABEL}>Every</span>
                <Input
                  id="alert-interval-amount"
                  aria-label="Check every"
                  {...describing("alert-interval", {
                    said: errors.interval_secs,
                  })}
                  type="number"
                  min={1}
                  step={1}
                  value={
                    Number.isNaN(form.interval.amount)
                      ? ""
                      : form.interval.amount
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
                <Choice
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
            label="Send to"
            required
            said={errors.destination}
          >
            <Choice
              id="alert-destination"
              value={form.destination}
              options={[
                ...(form.destination === ""
                  ? [{ value: "", label: "Pick a destination" }]
                  : []),
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
          <span className="text-sm">Run checks</span>
        </label>
      </Section>
    </div>
  );
}
