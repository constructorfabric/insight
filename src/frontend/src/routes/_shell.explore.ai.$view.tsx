import { createFileRoute, redirect } from "@tanstack/react-router";

import { AiCostView } from "@/components/portal/ai-cost-view";
import { resolveZoneItem } from "@/lib/portal/nav-model";
import { aiItem } from "@/lib/portal/portal-routes";

export const Route = createFileRoute("/_shell/explore/ai/$view")({
  beforeLoad: ({ params }) => {
    if (!aiItem(params.view)) throw redirect({ to: "/explore/ai", replace: true });
  },
  staticData: { zone: "aicost" },
  component: AiView,
});

function AiView() {
  const { view } = Route.useParams();
  return <AiCostView item={resolveZoneItem("aicost", aiItem(view) ?? null)} />;
}
