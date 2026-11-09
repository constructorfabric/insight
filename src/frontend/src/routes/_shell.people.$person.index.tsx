import { createFileRoute } from "@tanstack/react-router";

import { PersonView } from "@/components/portal/person-view";

export const Route = createFileRoute("/_shell/people/$person/")({
  staticData: { zone: "person" },
  component: PersonPage,
});

function PersonPage() {
  const { person } = Route.useParams();
  return <PersonView person={person} section={null} />;
}
