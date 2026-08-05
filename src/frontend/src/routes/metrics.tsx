import { createFileRoute } from "@tanstack/react-router";

import { MetricDefinitionsScreen } from "@/screens/metric-definitions";

export const Route = createFileRoute("/metrics")({
  component: MetricDefinitionsScreen,
});
