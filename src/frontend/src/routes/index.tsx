import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/")({
  // "/" is not a page — it is a redirect into the portal, so the address bar
  // names a real destination from the first paint and Back out of the portal
  // leaves the app rather than looping.
  beforeLoad: () => {
    throw redirect({ to: "/portal" });
  },
});
