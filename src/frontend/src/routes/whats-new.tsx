import { createFileRoute } from "@tanstack/react-router";

import { WhatsNewScreen } from "@/screens/whats-new";

export const Route = createFileRoute("/whats-new")({
  component: WhatsNewScreen,
});
