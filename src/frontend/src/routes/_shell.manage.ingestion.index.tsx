import { createFileRoute } from "@tanstack/react-router";

import { ManageView } from "@/components/portal/manage-view";

export const Route = createFileRoute("/_shell/manage/ingestion/")({
  staticData: { zone: "manage", item: "ingestion" },
  component: () => <ManageView item="ingestion" />,
});
