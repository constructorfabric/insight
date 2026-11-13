import { createFileRoute } from "@tanstack/react-router";

import { ManageView } from "@/components/portal/manage-view";
import { validateIdentitiesSearch } from "@/lib/portal/portal-search";

export const Route = createFileRoute("/_shell/manage/identities/")({
  validateSearch: validateIdentitiesSearch,
  staticData: { zone: "manage", item: "identities" },
  component: () => <ManageView item="identities" />,
});
