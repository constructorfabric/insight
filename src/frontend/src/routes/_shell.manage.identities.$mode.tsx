import { createFileRoute, redirect } from "@tanstack/react-router";

import { ManageView } from "@/components/portal/manage-view";
import { MODES } from "@/lib/portal/identity-modes";
import { validateIdentitiesSearch } from "@/lib/portal/portal-search";

export const Route = createFileRoute("/_shell/manage/identities/$mode")({
  validateSearch: validateIdentitiesSearch,
  beforeLoad: ({ params }) => {
    if (!MODES.some((mode) => mode === params.mode)) {
      throw redirect({ to: "/manage/identities", replace: true });
    }
  },
  staticData: { zone: "manage", item: "identities" },
  component: IdentitiesPage,
});

function IdentitiesPage() {
  const { mode } = Route.useParams();
  return <ManageView item="identities" mode={mode} />;
}
