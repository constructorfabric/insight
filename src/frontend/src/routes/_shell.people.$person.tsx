import { Outlet, createFileRoute, redirect } from "@tanstack/react-router";

import { isPersonId, normalizePersonId } from "@/lib/metrics/entity";

export const Route = createFileRoute("/_shell/people/$person")({
  // A non-person-id `$person` (the nil UUID, an email, a typo) is a 400 from
  // the metrics API that the reader cannot act on.
  beforeLoad: ({ params, location }) => {
    if (!isPersonId(params.person)) throw redirect({ to: "/", replace: true });
    // One spelling per person: the pane, the page and the metric requests all
    // key on the normalised id.
    const person = normalizePersonId(params.person);
    if (person !== params.person) {
      // A person id is a UUID, so the casing is the only difference to rewrite.
      throw redirect({ href: location.href.replace(params.person, person), replace: true });
    }
  },
  component: Outlet,
});
