import { createFileRoute, redirect } from "@tanstack/react-router";

import { getViewerPersonId } from "@/auth";
import { bareLandingZone } from "@/lib/portal/landing-zone";
import { getPortalShowPlanned } from "@/lib/portal/portal-store";
import { linkTo } from "@/lib/portal/portal-routes";

/** The landing page: Home, or the reader's own page where the install leaves Home out. */
export const Route = createFileRoute("/")({
  beforeLoad: () => {
    const person = getViewerPersonId() ?? "";
    const zone = bareLandingZone(getPortalShowPlanned());
    throw redirect(linkTo(zone === "overview" ? { zone } : { zone, person }));
  },
});
