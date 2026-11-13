import { createFileRoute, redirect } from "@tanstack/react-router";

import { getViewerPersonId } from "@/auth";
import { bareLandingZone } from "@/lib/portal/landing-zone";
import { getPortalShowPlanned } from "@/lib/portal/portal-store";
import { linkTo } from "@/lib/portal/portal-routes";

/** The landing page: Home, or the reader's own page where the install leaves Home out. */
export const Route = createFileRoute("/")({
  beforeLoad: () => {
    const person = getViewerPersonId();
    const zone = bareLandingZone(getPortalShowPlanned());
    if (zone === "overview") throw redirect(linkTo({ zone }));
    // A session that names no person has no page of its own to land on.
    throw redirect(person ? linkTo({ zone, person }) : { to: "/dashboards" });
  },
});
