import { createFileRoute, redirect } from "@tanstack/react-router";

import { ManageView } from "@/components/portal/manage-view";

/** A bronze connector slug, as the endpoint's `scope` validator accepts it without the prefix. */
const CONNECTOR_SLUG = /^[a-z0-9_]{1,120}$/;

export const Route = createFileRoute("/_shell/manage/ingestion/$connector")({
  beforeLoad: ({ params }) => {
    if (!CONNECTOR_SLUG.test(params.connector)) {
      throw redirect({ to: "/manage/ingestion", replace: true });
    }
  },
  staticData: { zone: "manage", item: "ingestion" },
  component: IngestionPage,
});

function IngestionPage() {
  const { connector } = Route.useParams();
  return <ManageView item="ingestion" connector={connector} />;
}
