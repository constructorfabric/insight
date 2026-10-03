import { createFileRoute, useRouterState } from "@tanstack/react-router";

import { EditAlertPage } from "@/components/alerts/alert-form";
import { alertIdFromPath } from "@/lib/alerts/route";

export const Route = createFileRoute("/portal/custom/alerts/$id/edit")({
  component: EditAlertRoute,
});

function EditAlertRoute() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });

  return <EditAlertPage id={alertIdFromPath(pathname)} />;
}
