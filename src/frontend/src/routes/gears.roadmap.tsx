import { createFileRoute } from "@tanstack/react-router";

import { GearRoadmapGridScreen } from "@/screens/gear-roadmap-grid";

export const Route = createFileRoute("/gears/roadmap")({
  component: GearRoadmapGridScreen,
});
