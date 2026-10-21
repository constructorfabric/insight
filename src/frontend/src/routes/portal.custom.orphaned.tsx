import { createFileRoute } from "@tanstack/react-router";

import { OrphanedDashboards } from "@/components/custom/orphaned-dashboards";

/** Static, so it wins over `$name` — see the note on the metrics route. */
export const Route = createFileRoute("/portal/custom/orphaned")({
  component: OrphanedDashboards,
});
