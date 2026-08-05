import { Navigate, createFileRoute } from "@tanstack/react-router";

import { DashboardScreen } from "@/screens/dashboard";
import { isPersonId } from "@/lib/metrics/entity";

export const Route = createFileRoute("/ic/$person/personal")({
  component: PersonalRoute,
});

function PersonalRoute() {
  const { person } = Route.useParams();
  // `$person` is a canonical person id since the identity cutover. Anything
  // else — a pre-cutover email URL, the nil UUID, a typo — goes to the root
  // dashboard rather than to the metrics API, where it is a 400 the user
  // cannot act on.
  if (!isPersonId(person)) {
    return <Navigate to="/" replace />;
  }
  return <DashboardScreen personId={person} />;
}
