import { useState } from "react";
import { useTranslation } from "react-i18next";

import type {
  MetricComputation,
  MetricDirection,
  MetricFormat,
} from "@/api/metrics-client";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { apiErrorReason } from "@/lib/query-console/api-error";
import {
  draftIsSubmittable,
  EMPTY_DRAFT,
  type MetricDraft,
} from "@/lib/metrics-console/metric-graph";
import { TriangleAlert } from "lucide-react";

const FORMATS: MetricFormat[] = ["integer", "decimal", "currency", "percent"];
const DIRECTIONS: MetricDirection[] = [
  "higher_is_better",
  "lower_is_better",
  "neutral",
];
const COMPUTATIONS: MetricComputation[] = [
  "sum",
  "ratio",
  "median",
  "distinct_count",
];

export interface MetricEditorDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  mode: "create" | "edit";
  /** Prefill for edit; ignored for create. */
  initial?: MetricDraft;
  onSubmit: (draft: MetricDraft) => void;
  isPending: boolean;
  error: unknown;
}

/** A styled native select — enum fields render fully in jsdom this way. */
function EnumSelect<T extends string>({
  id,
  value,
  options,
  onChange,
}: {
  id: string;
  value: T;
  options: readonly T[];
  onChange: (value: T) => void;
}) {
  return (
    <select
      id={id}
      value={value}
      onChange={(event) => onChange(event.target.value as T)}
      className="h-9 rounded-md border border-input bg-transparent px-2.5 text-sm shadow-xs outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
    >
      {options.map((option) => (
        <option key={option} value={option}>
          {option}
        </option>
      ))}
    </select>
  );
}

export function MetricEditorDialog({
  open,
  onOpenChange,
  mode,
  initial,
  onSubmit,
  isPending,
  error,
}: MetricEditorDialogProps) {
  const { t } = useTranslation();
  // The parent mounts this dialog only while open, so the initializer runs
  // fresh on each open: create starts blank, edit prefills from `initial`.
  const [draft, setDraft] = useState<MetricDraft>(initial ?? EMPTY_DRAFT);

  const set = <K extends keyof MetricDraft>(key: K, value: MetricDraft[K]) =>
    setDraft((d) => ({ ...d, [key]: value }));

  const canSubmit = draftIsSubmittable(draft) && !isPending;
  const isRatio = draft.computation === "ratio";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            {mode === "create"
              ? t("metrics_console.editor.create_title")
              : t("metrics_console.editor.edit_title")}
          </DialogTitle>
        </DialogHeader>

        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            if (canSubmit) onSubmit(draft);
          }}
        >
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <Field
              id="metric-key"
              label={t("metrics_console.editor.metric_key_label")}
            >
              <Input
                id="metric-key"
                value={draft.metric_key}
                onChange={(e) => set("metric_key", e.target.value)}
                placeholder="family.name"
                autoComplete="off"
                disabled={mode === "edit"}
              />
            </Field>
            <Field
              id="metric-label"
              label={t("metrics_console.editor.label_label")}
            >
              <Input
                id="metric-label"
                value={draft.label}
                onChange={(e) => set("label", e.target.value)}
                autoComplete="off"
              />
            </Field>
            <Field
              id="metric-short-label"
              label={t("metrics_console.editor.short_label_label")}
            >
              <Input
                id="metric-short-label"
                value={draft.short_label}
                onChange={(e) => set("short_label", e.target.value)}
                autoComplete="off"
              />
            </Field>
            <Field
              id="metric-entity-type"
              label={t("metrics_console.editor.entity_type_label")}
            >
              <Input
                id="metric-entity-type"
                value={draft.entity_type}
                onChange={(e) => set("entity_type", e.target.value)}
                placeholder="person"
                autoComplete="off"
              />
            </Field>
          </div>

          <Field
            id="metric-description"
            label={t("metrics_console.editor.description_label")}
          >
            <Input
              id="metric-description"
              value={draft.description}
              onChange={(e) => set("description", e.target.value)}
              autoComplete="off"
            />
          </Field>

          <Field
            id="metric-explanation"
            label={t("metrics_console.editor.explanation_label")}
          >
            <Textarea
              id="metric-explanation"
              value={draft.explanation}
              onChange={(e) => set("explanation", e.target.value)}
              className="min-h-16"
            />
          </Field>

          <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
            <Field
              id="metric-format"
              label={t("metrics_console.editor.format_label")}
            >
              <EnumSelect
                id="metric-format"
                value={draft.format}
                options={FORMATS}
                onChange={(v) => set("format", v)}
              />
            </Field>
            <Field
              id="metric-direction"
              label={t("metrics_console.editor.direction_label")}
            >
              <EnumSelect
                id="metric-direction"
                value={draft.direction}
                options={DIRECTIONS}
                onChange={(v) => set("direction", v)}
              />
            </Field>
            <Field
              id="metric-computation"
              label={t("metrics_console.editor.computation_label")}
            >
              <EnumSelect
                id="metric-computation"
                value={draft.computation}
                options={COMPUTATIONS}
                onChange={(v) => set("computation", v)}
              />
            </Field>
          </div>

          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <Field
              id="metric-unit"
              label={t("metrics_console.editor.unit_label")}
            >
              <Input
                id="metric-unit"
                value={draft.unit}
                onChange={(e) => set("unit", e.target.value)}
                autoComplete="off"
              />
            </Field>
            {isRatio ? (
              <Field
                id="metric-scale"
                label={t("metrics_console.editor.scale_label")}
              >
                <Input
                  id="metric-scale"
                  value={draft.scale}
                  onChange={(e) => set("scale", e.target.value)}
                  inputMode="decimal"
                  placeholder="100"
                  autoComplete="off"
                />
              </Field>
            ) : null}
            <Field
              id="metric-peer-cohort-key"
              label={t("metrics_console.editor.peer_cohort_key_label")}
            >
              <Input
                id="metric-peer-cohort-key"
                value={draft.peer_cohort_key}
                onChange={(e) => set("peer_cohort_key", e.target.value)}
                autoComplete="off"
              />
            </Field>
          </div>

          <Field
            id="metric-source-key"
            label={t("metrics_console.editor.source_key_label")}
          >
            <Input
              id="metric-source-key"
              value={draft.source_key}
              onChange={(e) => set("source_key", e.target.value)}
              placeholder="example_source"
              autoComplete="off"
            />
          </Field>

          <Field
            id="metric-observation-sql"
            label={t("metrics_console.editor.observation_sql_label")}
          >
            <Textarea
              id="metric-observation-sql"
              value={draft.observation_sql}
              onChange={(e) => set("observation_sql", e.target.value)}
              className="min-h-40 font-mono text-xs"
              spellCheck={false}
              placeholder={t(
                "metrics_console.editor.observation_sql_placeholder"
              )}
            />
          </Field>

          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <Field
              id="metric-measures"
              label={t("metrics_console.editor.measures_label")}
            >
              <Input
                id="metric-measures"
                value={draft.measures}
                onChange={(e) => set("measures", e.target.value)}
                placeholder="lines, commits"
                autoComplete="off"
              />
            </Field>
            <Field
              id="metric-dimensions"
              label={t("metrics_console.editor.dimensions_label")}
            >
              <Input
                id="metric-dimensions"
                value={draft.dimensions}
                onChange={(e) => set("dimensions", e.target.value)}
                placeholder="repo, language"
                autoComplete="off"
              />
            </Field>
          </div>

          {isRatio ? (
            <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
              <Field
                id="metric-numerator"
                label={t("metrics_console.editor.numerator_label")}
              >
                <Input
                  id="metric-numerator"
                  value={draft.numerator_measure}
                  onChange={(e) => set("numerator_measure", e.target.value)}
                  autoComplete="off"
                />
              </Field>
              <Field
                id="metric-denominator"
                label={t("metrics_console.editor.denominator_label")}
              >
                <Input
                  id="metric-denominator"
                  value={draft.denominator_measure}
                  onChange={(e) => set("denominator_measure", e.target.value)}
                  autoComplete="off"
                />
              </Field>
            </div>
          ) : (
            <Field
              id="metric-value-measure"
              label={t("metrics_console.editor.value_measure_label")}
            >
              <Input
                id="metric-value-measure"
                value={draft.value_measure}
                onChange={(e) => set("value_measure", e.target.value)}
                autoComplete="off"
              />
            </Field>
          )}

          <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
            <Field
              id="metric-transform-multiplier"
              label={t("metrics_console.editor.multiplier_label")}
            >
              <Input
                id="metric-transform-multiplier"
                value={draft.transform_multiplier}
                onChange={(e) => set("transform_multiplier", e.target.value)}
                inputMode="decimal"
                autoComplete="off"
              />
            </Field>
            <Field
              id="metric-transform-offset"
              label={t("metrics_console.editor.offset_label")}
            >
              <Input
                id="metric-transform-offset"
                value={draft.transform_offset}
                onChange={(e) => set("transform_offset", e.target.value)}
                inputMode="decimal"
                autoComplete="off"
              />
            </Field>
            <Field
              id="metric-transform-clamp-min"
              label={t("metrics_console.editor.clamp_min_label")}
            >
              <Input
                id="metric-transform-clamp-min"
                value={draft.transform_clamp_min}
                onChange={(e) => set("transform_clamp_min", e.target.value)}
                inputMode="decimal"
                autoComplete="off"
              />
            </Field>
            <Field
              id="metric-transform-clamp-max"
              label={t("metrics_console.editor.clamp_max_label")}
            >
              <Input
                id="metric-transform-clamp-max"
                value={draft.transform_clamp_max}
                onChange={(e) => set("transform_clamp_max", e.target.value)}
                inputMode="decimal"
                autoComplete="off"
              />
            </Field>
          </div>

          {error ? (
            <Alert variant="destructive">
              <TriangleAlert />
              <AlertDescription>
                {apiErrorReason(error, t("metrics_console.editor.save_failed"))}
              </AlertDescription>
            </Alert>
          ) : null}

          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              onClick={() => onOpenChange(false)}
            >
              {t("metrics_console.editor.cancel")}
            </Button>
            <Button type="submit" disabled={!canSubmit}>
              {t("metrics_console.editor.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function Field({
  id,
  label,
  children,
}: {
  id: string;
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      {children}
    </div>
  );
}
