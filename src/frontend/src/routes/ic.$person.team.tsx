import { createFileRoute, redirect } from "@tanstack/react-router";

import { isPersonId } from "@/lib/metrics/entity";

export const Route = createFileRoute("/ic/$person/team")({
  // A non-person-id `$person` (a pre-cutover email URL, the nil UUID, a typo)
  // is a 400 from the metrics API that the reader cannot act on.
  beforeLoad: ({ params }) => {
    if (!isPersonId(params.person)) throw redirect({ to: "/portal" });
  },
});
