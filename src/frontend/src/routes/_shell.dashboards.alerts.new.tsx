import { createFileRoute } from "@tanstack/react-router";

import { NewAlertPage } from "@/components/alerts/alert-form";

export const Route = createFileRoute("/_shell/dashboards/alerts/new")({
  component: NewAlertPage,
});
