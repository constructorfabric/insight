import { createFileRoute } from "@tanstack/react-router";

import { PeopleView } from "@/components/portal/people-view";

export const Route = createFileRoute("/_shell/people/$person/team/")({
  staticData: { zone: "people" },
  component: TeamPage,
});

function TeamPage() {
  const { person } = Route.useParams();
  return <PeopleView person={person} item="roster" />;
}
