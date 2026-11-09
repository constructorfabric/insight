import { Outlet, createFileRoute, redirect } from "@tanstack/react-router";

import { isPersonId } from "@/lib/metrics/entity";

export const Route = createFileRoute("/_shell/people/$person")({
  // A non-person-id `$person` (the nil UUID, an email, a typo) is a 400 from
  // the metrics API that the reader cannot act on.
  beforeLoad: ({ params }) => {
    if (!isPersonId(params.person)) throw redirect({ to: "/", replace: true });
  },
  component: Outlet,
});
