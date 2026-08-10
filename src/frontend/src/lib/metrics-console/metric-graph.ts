/**
 * Conversions between the authoring form's flat string draft and the wire
 * `CustomMetricGraph`. The editor keeps every field as a string (list fields
 * as comma/newline text, numbers as text) so the form stays controlled and
 * type-free; these helpers parse a draft into a graph on submit and hydrate a
 * draft from a stored graph on edit.
 */

import type {
  CustomMetricGraph,
  MetricComputation,
  MetricDirection,
  MetricFormat,
  MetricInput,
  MetricTransform,
} from "@/api/metrics-client";

export interface MetricDraft {
  metric_key: string;
  label: string;
  short_label: string;
  description: string;
  explanation: string;
  entity_type: string;
  unit: string;
  format: MetricFormat;
  direction: MetricDirection;
  computation: MetricComputation;
  scale: string;
  peer_cohort_key: string;
  transform_multiplier: string;
  transform_offset: string;
  transform_clamp_min: string;
  transform_clamp_max: string;
  source_key: string;
  observation_sql: string;
  /** Comma/newline separated measure keys. */
  measures: string;
  /** Comma/newline separated dimension keys. */
  dimensions: string;
  /** sum / median / distinct_count: the single value measure. */
  value_measure: string;
  /** ratio: numerator measure. */
  numerator_measure: string;
  /** ratio: denominator measure. */
  denominator_measure: string;
}

export const EMPTY_DRAFT: MetricDraft = {
  metric_key: "",
  label: "",
  short_label: "",
  description: "",
  explanation: "",
  entity_type: "person",
  unit: "",
  format: "integer",
  direction: "neutral",
  computation: "sum",
  scale: "",
  peer_cohort_key: "",
  transform_multiplier: "",
  transform_offset: "",
  transform_clamp_min: "",
  transform_clamp_max: "",
  source_key: "",
  observation_sql: "",
  measures: "",
  dimensions: "",
  value_measure: "",
  numerator_measure: "",
  denominator_measure: "",
};

function splitList(text: string): string[] {
  return text
    .split(/[\n,]/)
    .map((token) => token.trim())
    .filter((token) => token !== "");
}

function toNullableString(text: string): string | null {
  const trimmed = text.trim();
  return trimmed === "" ? null : trimmed;
}

function toNullableNumber(text: string): number | null {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : null;
}

function buildTransform(draft: MetricDraft): MetricTransform | null {
  const transform: MetricTransform = {
    multiplier: toNullableNumber(draft.transform_multiplier),
    offset: toNullableNumber(draft.transform_offset),
    clamp_min: toNullableNumber(draft.transform_clamp_min),
    clamp_max: toNullableNumber(draft.transform_clamp_max),
  };
  const hasAny = Object.values(transform).some((v) => v !== null);
  return hasAny ? transform : null;
}

function buildInputs(draft: MetricDraft): MetricInput[] {
  if (draft.computation === "ratio") {
    return [
      { role: "numerator", measure_key: draft.numerator_measure.trim() },
      { role: "denominator", measure_key: draft.denominator_measure.trim() },
    ];
  }
  return [{ role: "value", measure_key: draft.value_measure.trim() }];
}

/** Parse a draft into the wire graph. `scale` is sent only for ratios. */
export function draftToGraph(draft: MetricDraft): CustomMetricGraph {
  const isRatio = draft.computation === "ratio";
  return {
    metric_key: draft.metric_key.trim(),
    label: draft.label.trim(),
    short_label: toNullableString(draft.short_label),
    description: toNullableString(draft.description),
    explanation: toNullableString(draft.explanation),
    entity_type: draft.entity_type.trim(),
    unit: toNullableString(draft.unit),
    format: draft.format,
    direction: draft.direction,
    computation: draft.computation,
    scale: isRatio ? toNullableNumber(draft.scale) : null,
    peer_cohort_key: toNullableString(draft.peer_cohort_key),
    transform: buildTransform(draft),
    source_key: draft.source_key.trim(),
    observation_sql: draft.observation_sql,
    measures: splitList(draft.measures),
    dimensions: splitList(draft.dimensions),
    inputs: buildInputs(draft),
  };
}

/** Hydrate a draft from a stored graph, for the edit form's prefill. */
export function graphToDraft(graph: CustomMetricGraph): MetricDraft {
  const numberToText = (v: number | null | undefined): string =>
    v === null || v === undefined ? "" : String(v);
  const inputFor = (role: MetricInput["role"]): string =>
    graph.inputs.find((input) => input.role === role)?.measure_key ?? "";
  return {
    metric_key: graph.metric_key,
    label: graph.label,
    short_label: graph.short_label ?? "",
    description: graph.description ?? "",
    explanation: graph.explanation ?? "",
    entity_type: graph.entity_type,
    unit: graph.unit ?? "",
    format: graph.format,
    direction: graph.direction,
    computation: graph.computation,
    scale: numberToText(graph.scale),
    peer_cohort_key: graph.peer_cohort_key ?? "",
    transform_multiplier: numberToText(graph.transform?.multiplier),
    transform_offset: numberToText(graph.transform?.offset),
    transform_clamp_min: numberToText(graph.transform?.clamp_min),
    transform_clamp_max: numberToText(graph.transform?.clamp_max),
    source_key: graph.source_key,
    observation_sql: graph.observation_sql,
    measures: graph.measures.join(", "),
    dimensions: graph.dimensions.join(", "),
    value_measure: inputFor("value"),
    numerator_measure: inputFor("numerator"),
    denominator_measure: inputFor("denominator"),
  };
}

/**
 * Minimum a draft needs before submit is allowed: identity, source, SQL, the
 * measures the SQL emits, and the measure wiring the computation requires
 * (both legs for a ratio, plus a scale).
 */
export function draftIsSubmittable(draft: MetricDraft): boolean {
  const baseOk =
    draft.metric_key.trim() !== "" &&
    draft.label.trim() !== "" &&
    draft.source_key.trim() !== "" &&
    draft.observation_sql.trim() !== "" &&
    splitList(draft.measures).length > 0;
  if (!baseOk) return false;
  if (draft.computation === "ratio") {
    return (
      draft.numerator_measure.trim() !== "" &&
      draft.denominator_measure.trim() !== "" &&
      // Scale is required for a ratio and must be a real number — the same
      // parse `draftToGraph` applies, so a non-numeric scale is not submittable.
      toNullableNumber(draft.scale) !== null
    );
  }
  return draft.value_measure.trim() !== "";
}
