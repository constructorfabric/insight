import { createFileRoute, redirect } from "@tanstack/react-router";

import { visibleLenses } from "@/lib/portal/lens-configs";
import { DIRECTIONS } from "@/lib/portal/nav-model";
import { getPortalShowPlanned } from "@/lib/portal/portal-store";
import { linkTo } from "@/lib/portal/portal-routes";

/** A direction opens on its first lens the reader can see. */
export const Route = createFileRoute("/_shell/explore/$dir/")({
  beforeLoad: ({ params }) => {
    const direction = DIRECTIONS.find((candidate) => candidate.id === params.dir);
    if (!direction) throw redirect({ to: "/explore", replace: true });
    const lens = visibleLenses(direction, getPortalShowPlanned())[0] ?? direction.lenses[0];
    throw redirect({ ...linkTo({ zone: "directions", dir: direction.id, lens }), replace: true });
  },
});
