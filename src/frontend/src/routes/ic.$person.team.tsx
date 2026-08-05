import { Navigate, createFileRoute } from "@tanstack/react-router";

import { isPersonId } from "@/lib/metrics/entity";
import { TeamViewScreen } from "@/screens/team-view";

export const Route = createFileRoute("/ic/$person/team")({
  component: TeamScreen,
});

function TeamScreen() {
  const { person } = Route.useParams();
  // Not a canonical person id (a pre-cutover email URL, the nil UUID, a typo):
  // the root dashboard, not the metrics API, where it is an unactionable 400.
  if (!isPersonId(person)) {
    return <Navigate to="/" replace />;
  }
  return <TeamViewScreen teamId={person} />;
}
