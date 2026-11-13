import { createFileRoute, redirect } from "@tanstack/react-router";

import { defaultZoneItem } from "@/lib/portal/nav-model";
import { linkTo } from "@/lib/portal/portal-routes";

/** Manage opens on the first surface every viewer can open. */
export const Route = createFileRoute("/_shell/manage/")({
  beforeLoad: () => {
    const item = defaultZoneItem("manage") ?? undefined;
    if (!item) throw redirect({ to: "/", replace: true });
    throw redirect({ ...linkTo({ zone: "manage", item }), replace: true });
  },
});
