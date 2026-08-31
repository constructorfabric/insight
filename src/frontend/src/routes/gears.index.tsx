import { createFileRoute } from "@tanstack/react-router";

import { GearOverviewScreen } from "@/screens/gear-overview";

export const Route = createFileRoute("/gears/")({
  component: GearOverviewScreen,
});
