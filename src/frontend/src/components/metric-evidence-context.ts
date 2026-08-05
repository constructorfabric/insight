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

export interface EvidenceDialogContextValue {
  openEvidence: (selection: MetricEvidenceSelection, label: string) => void;
  openEvidenceTargets: (
    targets: readonly EvidenceDialogTarget[],
    title?: string
  ) => void;
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
