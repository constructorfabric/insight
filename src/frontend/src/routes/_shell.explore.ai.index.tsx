import { createFileRoute } from "@tanstack/react-router";

import { AiCostView } from "@/components/portal/ai-cost-view";

export const Route = createFileRoute("/_shell/explore/ai/")({
  staticData: { zone: "aicost" },
  component: () => <AiCostView item="overview" />,
});
