import { createFileRoute, redirect } from "@tanstack/react-router";

import { PeopleView } from "@/components/portal/people-view";
import { resolveZoneItem } from "@/lib/portal/nav-model";
import { validateMetricsViewSearch } from "@/lib/portal/portal-search";
import { teamItem } from "@/lib/portal/portal-routes";

export const Route = createFileRoute("/_shell/people/$person/team/$view")({
  validateSearch: validateMetricsViewSearch,
  beforeLoad: ({ params }) => {
    if (!teamItem(params.view)) {
      throw redirect({ to: "/people/$person/team", params: { person: params.person }, replace: true });
    }
  },
  staticData: { zone: "people" },
  component: TeamView,
});

function TeamView() {
  const { person, view } = Route.useParams();
  return <PeopleView person={person} item={resolveZoneItem("people", teamItem(view) ?? null)} />;
}
