import { createFileRoute, redirect } from "@tanstack/react-router";

import { useViewer } from "@/auth";
import { readLegacyShell } from "@/lib/portal/portal-store";
import { FullScreenLoading } from "@/components/full-screen-loading";
import { DashboardScreen } from "@/screens/dashboard";

export const Route = createFileRoute("/")({
  // "/" is not a page — it is a redirect into the portal, so the address bar
  // names a real destination from the first paint and Back out of the portal
  // leaves the app rather than looping. The component below is reached only
  // under the legacy-shell hatch (see `readLegacyShell`).
  beforeLoad: () => {
    if (!readLegacyShell()) throw redirect({ to: "/portal" });
  },
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
