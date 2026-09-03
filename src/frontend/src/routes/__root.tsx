import { Outlet, createRootRoute } from "@tanstack/react-router";

import { TooltipProvider } from "@/components/ui/tooltip";
import { getPerson } from "@/api/identity-client";
import { authStore, getViewerPersonId, signIn } from "@/auth";
import { AuthGate } from "@/components/auth-gate";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { Toaster } from "@/components/ui/sonner";
import { normalizePersonId } from "@/lib/metrics/entity";
import { queryClient } from "@/query-client";
import { FeedbackDialogProvider } from "@/components/feedback-dialog-provider";
import { MetricEvidenceDialogProvider } from "@/components/metric-evidence-dialog-provider";

// Warms the exact key `useIcPerson` reads, so the shell mounts with the
// viewer's canonical person already cached.
export async function prefetchViewerIdentity(): Promise<void> {
  const personId = getViewerPersonId();
  if (!personId) return;
  await queryClient.prefetchQuery({
    queryKey: ["identity", "person", normalizePersonId(personId)],
    queryFn: ({ signal }) => getPerson(personId, signal),
  });
}

export const Route = createRootRoute({
  beforeLoad: async () => {
    // The session was probed once at boot (main.tsx → loadSession), so the
    // store is already resolved here. No client-side token dance — an absent
    // session means a full-page bounce to the gateway's login flow.
    const { status } = authStore.getSnapshot();
    if (status === "authenticated") {
      await prefetchViewerIdentity();
      return;
    }
    signIn(window.location.pathname + window.location.search);
  },
  component: RootLayout,
  // Shown while beforeLoad resolves auth + the viewer identity — the app
  // shell (sidebar, headers) mounts only once identity is cached, so no
  // surface ever renders a raw email or an empty org tree. `pendingMs: 0`
  // spins immediately instead of a blank first second.
  pendingComponent: RootPending,
  pendingMs: 0,
});

function RootPending() {
  return <CenteredSpinner className="min-h-screen w-full" />;
}

function RootLayout() {
  return (
    <TooltipProvider>
      <MetricEvidenceDialogProvider>
        <FeedbackDialogProvider>
          <AuthGate>
            <Outlet />
          </AuthGate>
          {/* Outside AuthGate: a toast must survive the surface that raised it
              closing, and the identity verbs report their result by closing the
              case window and toasting. Without this mount every `toast()` call
              in the app is silently dropped. */}
          <Toaster />
        </FeedbackDialogProvider>
      </MetricEvidenceDialogProvider>
    </TooltipProvider>
  );
}
