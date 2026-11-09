import { createFileRoute } from "@tanstack/react-router";

import { DirectionView } from "@/components/portal/direction-view";

export const Route = createFileRoute("/_shell/explore/")({
  staticData: { zone: "directions" },
  component: () => <DirectionView dir="" lens="" />,
});
