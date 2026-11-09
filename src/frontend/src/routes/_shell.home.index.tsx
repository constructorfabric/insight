import { createFileRoute } from "@tanstack/react-router";

import { OverviewView } from "@/components/portal/overview-view";

export const Route = createFileRoute("/_shell/home/")({
  staticData: { zone: "overview" },
  component: () => <OverviewView item="at-a-glance" />,
});
