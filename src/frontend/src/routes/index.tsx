import { createFileRoute } from "@tanstack/react-router";

import { useViewer } from "@/auth";
import { FullScreenLoading } from "@/components/full-screen-loading";
import { DashboardScreen } from "@/screens/dashboard";

export const Route = createFileRoute("/")({
  component: IndexRoute,
});

function IndexRoute() {
  const { personId } = useViewer();
  // An authenticated session always carries the person id (the gateway JWT
  // `sub`); the loading fallback only shows in the brief window before the
  // store resolves.
  if (!personId) return <FullScreenLoading />;
  return <DashboardScreen personId={personId} />;
}
