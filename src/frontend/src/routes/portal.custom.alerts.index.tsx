import { createFileRoute } from "@tanstack/react-router";

import { AlertsList } from "@/components/alerts/alerts-list";

export const Route = createFileRoute("/portal/custom/alerts/")({
  component: AlertsList,
});
