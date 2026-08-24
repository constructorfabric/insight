import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useQueryClient } from "@tanstack/react-query";

import { sessionAuthorizationScope } from "@/auth/session-scope";
import { useAuth } from "@/auth/use-auth";
import {
  EvidenceDialogContext,
  type EvidenceDialogOptions,
  type EvidenceDialogState,
  type EvidenceDialogTarget,
  type EvidencePeopleView,
} from "@/components/metric-evidence-context";
import { MetricEvidenceDialog } from "@/components/metric-evidence-dialog";
import { recordUsageEvent } from "@/telemetry";

type ScopedEvidenceDialogState = EvidenceDialogState & {
  sessionScope: string | null;
};

export function MetricEvidenceDialogProvider({
  children,
}: {
  children: ReactNode;
}) {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  const queryClient = useQueryClient();
  const previousSessionScope = useRef(sessionScope);
  const [state, setState] = useState<ScopedEvidenceDialogState | null>(null);
  useEffect(() => {
    if (previousSessionScope.current !== sessionScope) {
      void queryClient.cancelQueries({ queryKey: ["metric-drilldown"] });
      queryClient.removeQueries({ queryKey: ["metric-drilldown"] });
      setState(null);
    }
    previousSessionScope.current = sessionScope;
  }, [queryClient, sessionScope]);
  const openEvidenceTargets = useCallback(
    (
      targets: readonly EvidenceDialogTarget[],
      options?: EvidenceDialogOptions
    ) => {
      const uniqueTargets = [
        ...new Map(
          targets.map((target) => [target.selection.metric_key, target])
        ).values(),
      ];
      const first = uniqueTargets[0];
      if (!first) return;
      const requested = options?.activeMetricKey;
      const active = uniqueTargets.some(
        (target) => target.selection.metric_key === requested
      )
        ? requested!
        : first.selection.metric_key;
      recordUsageEvent("drill", active);
      setState({
        kind: "records",
        targets: [first, ...uniqueTargets.slice(1)],
        activeMetricKey: active,
        title: options?.title,
        sessionScope,
      });
    },
    [sessionScope]
  );
  const openEvidence = useCallback(
    (
      selection: EvidenceDialogTarget["selection"],
      label: string
    ) => openEvidenceTargets([{ selection, label }]),
    [openEvidenceTargets]
  );
  const openEvidencePeople = useCallback(
    (view: EvidencePeopleView) => {
      if (!view.rows.length) return;
      // Same usage event as a records drill: it counts a reader going one step
      // down from a figure, and under the same metric key, so the series stays
      // comparable whether or not the metric can be drilled to records.
      recordUsageEvent("drill", view.metricKey);
      setState({ kind: "people", view, sessionScope });
    },
    [sessionScope]
  );
  const selectEvidenceMetric = useCallback((metricKey: string) => {
    setState((current) =>
      current?.kind === "records" &&
      current.targets.some((target) => target.selection.metric_key === metricKey)
        ? { ...current, activeMetricKey: metricKey }
        : current
    );
  }, []);
  const value = useMemo(
    () => ({ openEvidence, openEvidenceTargets, openEvidencePeople }),
    [openEvidence, openEvidenceTargets, openEvidencePeople]
  );
  const visibleState = state?.sessionScope === sessionScope ? state : null;
  return (
    <EvidenceDialogContext.Provider value={value}>
      {children}
      <MetricEvidenceDialog
        key={sessionScope ?? "no-session"}
        state={visibleState}
        onMetricChange={selectEvidenceMetric}
        onClose={() => setState(null)}
      />
    </EvidenceDialogContext.Provider>
  );
}
