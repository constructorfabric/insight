import { createFileRoute } from "@tanstack/react-router";

import { Orphaned } from "@/components/custom/orphaned";

/** Static, so it wins over `$name` — see the note on the metrics route. */
export const Route = createFileRoute("/portal/custom/orphaned")({
  component: Orphaned,
});
