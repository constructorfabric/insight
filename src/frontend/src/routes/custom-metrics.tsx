import { createFileRoute } from "@tanstack/react-router";

import { MetricsConsoleScreen } from "@/screens/metrics-console";

export const Route = createFileRoute("/custom-metrics")({
  component: MetricsConsoleScreen,
});
