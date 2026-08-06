import { Outlet, createRootRoute, useRouterState } from "@tanstack/react-router";

import { TooltipProvider } from "@/components/ui/tooltip";
import { getPerson } from "@/api/identity-client";
import { authStore, getViewerPersonId, signIn } from "@/auth";
import { AppSidebar } from "@/components/app-sidebar";
import { AuthGate } from "@/components/auth-gate";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { MockBanner } from "@/components/mock-banner";
import { ViewAsBanner } from "@/components/view-as-banner";
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";
import { PortalLayout } from "@/components/portal/portal-layout";
import { isPortalShellPath } from "@/lib/portal/portal-routes";
import { usePortalEnabled } from "@/lib/portal/portal-store";
import { normalizePersonId } from "@/lib/metrics/entity";
import { queryClient } from "@/query-client";
import { MetricEvidenceDialogProvider } from "@/components/metric-evidence-dialog-provider";

// Warms the exact key `useIcPerson` reads, so the shell mounts with the
// viewer's tree already cached. Keyed by person_id since the identity cutover:
// an email here would both miss that key and make identity answer 400.
export async function prefetchViewerIdentity(): Promise<void> {
  const personId = getViewerPersonId();
  if (!personId) return;
  await queryClient.prefetchQuery({
    queryKey: ["identity", "person", normalizePersonId(personId)],
    queryFn: () => getPerson(personId),
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
  const portal = usePortalEnabled();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  // The portal is a ROUTE now, so it renders through the Outlet like anything
  // else — otherwise its navigation could never live in the URL. It still owns
  // the whole shell on the routes it claims; `isPortalShellPath` owns that list.
  const portalRoute = isPortalShellPath(pathname);
  return (
    <TooltipProvider>
      {/* Upstream's evidence-dialog provider wraps everything; the portal
          branch lives inside it, so a drilldown opened from a portal surface
          finds the same provider the legacy screens use. */}
      <MetricEvidenceDialogProvider>
        <AuthGate>
          {portal && portalRoute ? (
            <PortalLayout />
          ) : (
            <SidebarProvider>
              <AppSidebar />
              <SidebarInset className="min-w-0 overflow-x-clip">
                <MockBanner />
                <ViewAsBanner />
                <Outlet />
              </SidebarInset>
            </SidebarProvider>
          )}
        </AuthGate>
      </MetricEvidenceDialogProvider>
    </TooltipProvider>
  );
}
