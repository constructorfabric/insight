/* eslint-disable react-refresh/only-export-components -- this is a render
   helper (returns the gate element or null for an early return), not a
   hot-reloadable component; the org views call it, they don't mount it. */
import type { ReactNode } from "react";

import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";

/** What the gate needs to decide between loading, empty, failed, and ready. */
export interface OrgScopeGateArgs {
  /** Viewer identity query — `isLoading` (not `isPending`), so a disabled query
   * (viewer email still unresolved) doesn't spin forever. */
  viewerLoading: boolean;
  viewerError: boolean;
  membersLoading: boolean;
  membersError: boolean;
  memberCount: number;
  gridPending: boolean;
  gridError: boolean;
  /** Shown when the roster resolved but is empty. */
  emptyLabel: string;
  onRetry: () => void;
}

/**
 * Loading / error / empty gate shared by the org-scoped portal views (Overview,
 * People, Directions, AI & Cost, Collaboration). Returns the element to render
 * in place of the view, or `null` to proceed.
 *
 * Centralising this keeps the views' gating identical and — the reason it
 * exists — surfaces backend failures (`isError`) as an explicit retry state
 * instead of masking a 500 as "this manager has no team" (empty roster) or a
 * fabricated all-zero dashboard (empty metric grid). A disabled query reports
 * `isPending: true` forever, so callers pass `isLoading` for the viewer/members
 * queries; the metric grid already gates its own `isPending` on `enabled`.
 */
export function orgScopeGate(a: OrgScopeGateArgs): ReactNode | null {
  if (a.viewerLoading || a.membersLoading)
    return <CenteredSpinner className="min-h-[60vh]" />;
  if (a.viewerError || a.membersError) return <GateCard error onRetry={a.onRetry} />;
  if (a.memberCount === 0) return <GateCard label={a.emptyLabel} />;
  if (a.gridPending) return <CenteredSpinner className="min-h-[60vh]" />;
  if (a.gridError) return <GateCard error onRetry={a.onRetry} />;
  return null;
}

function GateCard({
  error,
  label,
  onRetry,
}: {
  error?: boolean;
  label?: string;
  onRetry?: () => void;
}) {
  return (
    <div className="mx-auto w-full max-w-md p-8">
      <ComingSoon
        variant="card"
        state={error ? "error" : "empty"}
        label={label}
        onRetry={onRetry}
      />
    </div>
  );
}
