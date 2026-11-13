import { createFileRoute, redirect } from "@tanstack/react-router";

import { OverviewView } from "@/components/portal/overview-view";
import { validateExploreSearch } from "@/lib/portal/portal-search";
import { resolveZoneItem } from "@/lib/portal/nav-model";
import { homeItem } from "@/lib/portal/portal-routes";

export const Route = createFileRoute("/_shell/home/$view")({
  // Home embeds lens views, which can open one repository.
  validateSearch: validateExploreSearch,
  beforeLoad: ({ params }) => {
    if (!homeItem(params.view)) throw redirect({ to: "/home", replace: true });
  },
  staticData: { zone: "overview" },
  component: HomeView,
});

function HomeView() {
  const { view } = Route.useParams();
  return <OverviewView item={resolveZoneItem("overview", homeItem(view) ?? null)} />;
}
