import { createFileRoute } from "@tanstack/react-router";

import { PreviewsScreen } from "@/screens/previews";

export const Route = createFileRoute("/previews")({
  component: PreviewsScreen,
});
