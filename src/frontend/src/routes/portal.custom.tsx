import {
  Outlet,
  createFileRoute,
  useNavigate,
  useRouterState,
} from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";

import type { ChatCreated } from "@/api/custom-client";
import { CustomChat } from "@/components/custom/custom-chat";
import { CustomPageShell } from "@/components/custom/custom-page-shell";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { dashboardIdFromPath } from "@/lib/custom/dashboard-path";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { useIsAdmin } from "@/queries/identity-me";
import {
  invalidateDashboardList,
  invalidateDashboardPage,
} from "@/queries/custom";

/**
 * The custom zone's layout: one assistant for the whole zone, beside whichever
 * page is open.
 *
 * The chat lives here rather than in each page because creating a dashboard
 * navigates to it — mounted per page, the panel unmounted mid-request and took
 * the conversation with it, so the reader lost what they had just asked.
 */
export const Route = createFileRoute("/portal/custom")({
  component: CustomZone,
});

const MEMBER_ROUTES = new Set(["/portal/custom/", "/portal/custom/$name"]);

function CustomZone() {
  const { isAdmin, isPending, isError, retry } = useIsAdmin();
  const showPlanned = usePortalShowPlanned();
  const routeId = useRouterState({ select: (s) => s.matches.at(-1)?.routeId });
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const openDashboard = dashboardIdFromPath(pathname);
  const queryClient = useQueryClient();
  const navigate = useNavigate();

  function handleCreated(created: ChatCreated) {
    void invalidateDashboardList(queryClient);
    // The page on screen may have just gained a widget rather than a whole
    // dashboard, so it is refreshed either way.
    if (openDashboard) {
      void invalidateDashboardPage(queryClient, openDashboard);
    }
    if (created.dashboard) {
      void invalidateDashboardPage(queryClient, created.dashboard.id);
      void navigate({
        to: "/portal/custom/$name",
        params: { name: created.dashboard.id },
      });
    }
  }

  if (isPending) return <CenteredSpinner className="min-h-40" />;
  if (isError) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon
          variant="card"
          state="error"
          label="Couldn't check your permissions."
          onRetry={retry}
        />
      </div>
    );
  }
  if (!isAdmin) {
    const open = routeId != null && MEMBER_ROUTES.has(routeId);

    return (
      <CustomPageShell>
        {open ? (
          <Outlet />
        ) : (
          <div className="mx-auto w-full max-w-md p-8">
            <ComingSoon
              variant="card"
              state="empty"
              label="This page is open to administrators only."
            />
          </div>
        )}
      </CustomPageShell>
    );
  }

  return (
    <CustomPageShell
      chat={showPlanned ? <CustomChat onCreated={handleCreated} /> : undefined}
    >
      <Outlet />
    </CustomPageShell>
  );
}
