import { createFileRoute, redirect } from "@tanstack/react-router";

import { defaultZoneItem } from "@/lib/portal/nav-model";
import { linkTo } from "@/lib/portal/portal-routes";

export const Route = createFileRoute("/_shell/reports/")({
  beforeLoad: () => {
    const item = defaultZoneItem("reports") ?? undefined;
    throw redirect({ ...linkTo({ zone: "reports", item }), replace: true });
  },
});
