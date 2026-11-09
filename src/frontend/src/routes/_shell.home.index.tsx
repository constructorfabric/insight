import { createFileRoute } from "@tanstack/react-router";

import { OverviewView } from "@/components/portal/overview-view";
import { validateExploreSearch } from "@/lib/portal/portal-search";

export const Route = createFileRoute("/_shell/home/")({
  // Home embeds lens views, which can open one repository.
  validateSearch: validateExploreSearch,
  staticData: { zone: "overview" },
  component: () => <OverviewView item="at-a-glance" />,
});
