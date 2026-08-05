import { createFileRoute } from "@tanstack/react-router";

import { QueryConsoleScreen } from "@/screens/query-console";

export const Route = createFileRoute("/queries")({
  component: QueryConsoleScreen,
});
