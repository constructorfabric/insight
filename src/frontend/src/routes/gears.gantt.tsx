import { createFileRoute } from "@tanstack/react-router";

import { GearGanttScreen } from "@/screens/gear-gantt";

export const Route = createFileRoute("/gears/gantt")({
  component: GearGanttScreen,
});
