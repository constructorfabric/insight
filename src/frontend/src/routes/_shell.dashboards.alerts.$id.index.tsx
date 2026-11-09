import { createFileRoute, useRouterState } from "@tanstack/react-router";

import { AlertPage } from "@/components/alerts/alert-page";
import { alertIdFromPath } from "@/lib/alerts/route";

export const Route = createFileRoute("/_shell/dashboards/alerts/$id/")({
  component: AlertRoute,
});

function AlertRoute() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });

  return <AlertPage id={alertIdFromPath(pathname)} />;
}
