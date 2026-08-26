import { createContext, useContext } from "react";

import type { MetricEvidenceSelection } from "@/api/metric-drilldown-client";

export interface EvidenceDialogTarget {
  selection: MetricEvidenceSelection;
  label: string;
}

/** One person behind an aggregate figure, and the records of their own value. */
export interface EvidencePersonRow {
  /** Entity id the metric is keyed by — also the row's identity. */
  entityId: string;
  /** Person id the Person zone is routed by; absent if the roster has none. */
  personId: string | null;
  name: string;
  value: number;
  valueText: string;
  /** Null when the metric carries no readable evidence: the row does not drill. */
  target: EvidenceDialogTarget | null;
}

/**
 * The people a figure was taken over — the drill step between an aggregate and
 * source records. Rows come from values the caller already holds, so this view
 * needs no request and always agrees with the figure that opened it.
 */
export interface EvidencePeopleView {
  /** Names the subset, e.g. "Commits · 100–150 commits per person". */
  title: string;
  /** The metric the figure came from — the key usage events are counted under. */
  metricKey: string;
  /** Column head over the values, e.g. "Commits". */
  valueLabel: string;
  rows: readonly EvidencePersonRow[];
  /** Every row's records at once; null when the set is too wide to read. */
  allRecords: EvidenceDialogTarget | null;
}

export type EvidenceDialogState =
  | {
      kind: "records";
      targets: readonly [EvidenceDialogTarget, ...EvidenceDialogTarget[]];
      activeMetricKey: string;
      title?: string;
    }
  | { kind: "people"; view: EvidencePeopleView };

export interface EvidenceDialogOptions {
  title?: string;
  activeMetricKey?: string;
}

export interface EvidenceDialogContextValue {
  openEvidence: (selection: MetricEvidenceSelection, label: string) => void;
  openEvidenceTargets: (
    targets: readonly EvidenceDialogTarget[],
    options?: EvidenceDialogOptions
  ) => void;
  /** The people behind an aggregate figure, one drill step above their records. */
  openEvidencePeople: (view: EvidencePeopleView) => void;
}

/**
 * The metrics a surface can offer alongside the one being opened, so every
 * entry lands in the same dialog with the same picker.
 */
export const EvidenceScopeContext = createContext<
  readonly EvidenceDialogTarget[]
>([]);

export function useEvidenceScope(): readonly EvidenceDialogTarget[] {
  return useContext(EvidenceScopeContext);
}

/**
 * The scope with `own` in place of its own metric, so the selection the caller
 * built — its filters and display dimensions — is the one that opens.
 */
export function withOwnTarget(
  scope: readonly EvidenceDialogTarget[],
  own: EvidenceDialogTarget
): EvidenceDialogTarget[] {
  const key = own.selection.metric_key;
  return scope.some((target) => target.selection.metric_key === key)
    ? scope.map((target) =>
        target.selection.metric_key === key ? own : target
      )
    : [own, ...scope];
}

export const EvidenceDialogContext = createContext<
  EvidenceDialogContextValue | undefined
>(undefined);

export function useMetricEvidence(): EvidenceDialogContextValue {
  const context = useContext(EvidenceDialogContext);
  if (!context) {
    throw new Error(
      "useMetricEvidence must be used within MetricEvidenceDialogProvider"
    );
  }
  return context;
}

export function useMetricEvidenceOptional():
  | EvidenceDialogContextValue
  | undefined {
  return useContext(EvidenceDialogContext);
}
