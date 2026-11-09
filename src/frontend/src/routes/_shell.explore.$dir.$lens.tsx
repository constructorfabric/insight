import { createFileRoute, redirect } from "@tanstack/react-router";

import { DirectionView } from "@/components/portal/direction-view";
import { validateExploreSearch } from "@/lib/portal/portal-search";
import { directionLens } from "@/lib/portal/portal-routes";

export const Route = createFileRoute("/_shell/explore/$dir/$lens")({
  validateSearch: validateExploreSearch,
  beforeLoad: ({ params }) => {
    if (!directionLens(params.dir, params.lens)) {
      throw redirect({ to: "/explore/$dir", params: { dir: params.dir }, replace: true });
    }
  },
  staticData: { zone: "directions" },
  component: LensView,
});

function LensView() {
  const { dir, lens } = Route.useParams();
  return <DirectionView dir={dir} lens={directionLens(dir, lens)?.lens ?? ""} />;
}
