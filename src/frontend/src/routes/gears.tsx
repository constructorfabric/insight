import { createFileRoute } from "@tanstack/react-router";

import { GearRoadmapLayout } from "@/screens/gear-roadmap-layout";

export const Route = createFileRoute("/gears")({
  component: GearRoadmapLayout,
});
