import { createContext, useContext } from "react";

import type { MetricEvidenceSelection } from "@/api/metric-drilldown-client";

export interface EvidenceDialogTarget {
  selection: MetricEvidenceSelection;
  label: string;
}

export interface EvidenceDialogState {
  targets: readonly [EvidenceDialogTarget, ...EvidenceDialogTarget[]];
  activeMetricKey: string;
  title?: string;
}

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
