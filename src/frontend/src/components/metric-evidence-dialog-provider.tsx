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
  type EvidenceDialogState,
} from "@/components/metric-evidence-context";
import { MetricEvidenceDialog } from "@/components/metric-evidence-dialog";

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
      targets: readonly EvidenceDialogState["targets"][number][],
      title?: EvidenceDialogState["title"]
    ) => {
      const uniqueTargets = [
        ...new Map(
          targets.map((target) => [target.selection.metric_key, target])
        ).values(),
      ];
      const first = uniqueTargets[0];
      if (!first) return;
      setState({
        targets: [first, ...uniqueTargets.slice(1)],
        activeMetricKey: first.selection.metric_key,
        title,
        sessionScope,
      });
    },
    [sessionScope]
  );
  const openEvidence = useCallback(
    (
      selection: EvidenceDialogState["targets"][number]["selection"],
      label: string
    ) => openEvidenceTargets([{ selection, label }]),
    [openEvidenceTargets]
  );
  const selectEvidenceMetric = useCallback((metricKey: string) => {
    setState((current) =>
      current?.targets.some(
        (target) => target.selection.metric_key === metricKey
      )
        ? { ...current, activeMetricKey: metricKey }
        : current
    );
  }, []);
  const value = useMemo(
    () => ({ openEvidence, openEvidenceTargets }),
    [openEvidence, openEvidenceTargets]
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
