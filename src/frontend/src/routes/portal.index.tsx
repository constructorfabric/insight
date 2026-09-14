import { createFileRoute } from "@tanstack/react-router";

import { ZoneContent } from "@/components/portal/zone-content";

/**
 * The portal's own zones, which live in `?zone=` rather than in the path. They
 * render here so the shell can host real child routes beside them.
 */
export const Route = createFileRoute("/portal/")({
  component: ZoneContent,
});
