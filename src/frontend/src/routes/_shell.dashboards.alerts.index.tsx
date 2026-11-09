import { createFileRoute } from "@tanstack/react-router";

import { AlertsList } from "@/components/alerts/alerts-list";

export const Route = createFileRoute("/_shell/dashboards/alerts/")({
  component: AlertsList,
});
