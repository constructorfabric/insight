import { createFileRoute, redirect } from "@tanstack/react-router";

import { ReportsView } from "@/components/portal/reports-view";
import { resolveZoneItem } from "@/lib/portal/nav-model";
import { reportItem } from "@/lib/portal/portal-routes";

export const Route = createFileRoute("/_shell/reports/$view")({
  beforeLoad: ({ params }) => {
    if (!reportItem(params.view)) throw redirect({ to: "/reports", replace: true });
  },
  staticData: { zone: "reports" },
  component: ReportPage,
});

function ReportPage() {
  const { view } = Route.useParams();
  return <ReportsView item={resolveZoneItem("reports", reportItem(view) ?? null)} />;
}
