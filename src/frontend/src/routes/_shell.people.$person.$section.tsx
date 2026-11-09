import { createFileRoute, redirect } from "@tanstack/react-router";

import { PersonView } from "@/components/portal/person-view";
import { GROUPS } from "@/lib/insight/groups";
import { sectionOf } from "@/lib/portal/portal-routes";

export const Route = createFileRoute("/_shell/people/$person/$section")({
  beforeLoad: ({ params }) => {
    if (!GROUPS.some((group) => group.id === sectionOf(params.section))) {
      throw redirect({ to: "/people/$person", params: { person: params.person }, replace: true });
    }
  },
  staticData: { zone: "person" },
  component: PersonSection,
});

function PersonSection() {
  const { person, section } = Route.useParams();
  return <PersonView person={person} section={sectionOf(section)} />;
}
