import { createFileRoute } from "@tanstack/react-router";

import { NewAlertPage } from "@/components/alerts/alert-form";

export const Route = createFileRoute("/portal/custom/alerts/new")({
  component: NewAlertPage,
});
