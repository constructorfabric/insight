import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/ic/$person/")({
  beforeLoad: ({ params }) => {
    throw redirect({
      to: "/ic/$person/personal",
      params: { person: params.person },
      replace: true,
    });
  },
});
