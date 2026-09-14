import { createFileRoute } from "@tanstack/react-router";

import { ZoneContent } from "@/components/portal/zone-content";

export const Route = createFileRoute("/ic/$person/personal")({
  component: ZoneContent,
});
