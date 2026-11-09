import { createFileRoute, redirect } from "@tanstack/react-router";

import { ManageView } from "@/components/portal/manage-view";
import { resolveZoneItem } from "@/lib/portal/nav-model";
import { manageItem } from "@/lib/portal/portal-routes";

export const Route = createFileRoute("/_shell/manage/$surface")({
  beforeLoad: ({ params }) => {
    if (!manageItem(params.surface)) throw redirect({ to: "/manage", replace: true });
  },
  staticData: { zone: "manage" },
  component: ManagePage,
});

function ManagePage() {
  const { surface } = Route.useParams();
  return <ManageView item={resolveZoneItem("manage", manageItem(surface) ?? null)} />;
}
